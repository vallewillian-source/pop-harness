use std::sync::{Arc, Mutex};

use poph_flows::Metrics as CacheMetrics;
use poph_types::{Counter, DomainResult, Gauge, Registry};

#[derive(Debug, Clone)]
pub struct FlowMetrics {
    pub cache_hits: Counter,
    pub cache_errors: Counter,
    pub cache_loads: Counter,
    pub cache_waits: Counter,
    pub evictions: Counter,
    pub protected_in_use: Counter,
    pub uncached_revisions: Counter,
    pub cached_revisions: Gauge,
    pub revisions_in_use: Gauge,
    pub cache_bytes: Gauge,
    pub started_executions: Counter,
    pub completed_executions: Counter,
    pub abandoned_executions: Counter,
    pub failed_executions: Counter,
    pub g9_started: Counter,
    pub g9_none: Counter,
    pub g9_below_threshold: Counter,
    pub g9_fallback: Counter,
    pub pending_effects: Counter,
    pub sent_effects: Counter,
    pub failed_effects: Counter,
    pub rule_errors: Counter,
    last_cache: Arc<Mutex<CacheMetrics>>,
}

impl FlowMetrics {
    pub fn register(registry: &Registry) -> DomainResult<Self> {
        Ok(Self {
            cache_hits: registry.counter(
                "poph_fluxos_cache_acertos_total",
                "Consultas de revisão de fluxo servidas pelo cache.",
            )?,
            cache_errors: registry.counter(
                "poph_fluxos_cache_erros_total",
                "Consultas de revisão de fluxo que não acharam a revisão pronta no cache.",
            )?,
            cache_loads: registry.counter(
                "poph_fluxos_cache_cargas_total",
                "Revisões de fluxo compiladas do script (uma compilação cada).",
            )?,
            cache_waits: registry.counter(
                "poph_fluxos_cache_esperas_total",
                "Consultas que esperaram a compilação de outra consulta da mesma revisão.",
            )?,
            evictions: registry.counter(
                "poph_fluxos_expulsoes_total",
                "Revisões de fluxo expulsas do cache por falta de espaço.",
            )?,
            protected_in_use: registry.counter(
                "poph_fluxos_cache_em_uso_poupadas_total",
                "Revisões em uso que não foram expulsas porque alguém ainda as segura.",
            )?,
            uncached_revisions: registry.counter(
                "poph_fluxos_cache_nao_cacheadas_total",
                "Revisões compiladas que não couberam no teto e não foram guardadas.",
            )?,
            cached_revisions: registry.gauge(
                "poph_fluxos_revisoes_em_cache",
                "Revisões de fluxo guardadas no cache agora.",
            )?,
            revisions_in_use: registry.gauge(
                "poph_fluxos_revisoes_em_uso",
                "Revisões de fluxo guardadas que alguém está usando agora.",
            )?,
            cache_bytes: registry.gauge(
                "poph_fluxos_cache_bytes",
                "Peso estimado, em bytes, das revisões de fluxo guardadas no cache.",
            )?,
            started_executions: registry.counter(
                "poph_fluxos_execucoes_iniciadas_total",
                "Execuções de fluxo iniciadas.",
            )?,
            completed_executions: registry.counter(
                "poph_fluxos_execucoes_concluidas_total",
                "Execuções de fluxo que chegaram a um fim.",
            )?,
            abandoned_executions: registry.counter(
                "poph_fluxos_execucoes_abandonadas_total",
                "Execuções de fluxo abandonadas.",
            )?,
            failed_executions: registry.counter(
                "poph_fluxos_execucoes_falhas_total",
                "Execuções de fluxo que pararam em falha (decisão em ciclo, teto de saltos ou regra que não avalia).",
            )?,
            g9_started: registry.counter(
                "poph_fluxos_g9_iniciou_total",
                "Decisões do G9 que iniciaram um fluxo.",
            )?,
            g9_none: registry.counter(
                "poph_fluxos_g9_nenhum_total",
                "Decisões do G9 em que o Jev respondeu que nenhum fluxo serve.",
            )?,
            g9_below_threshold: registry.counter(
                "poph_fluxos_g9_abaixo_do_limiar_total",
                "Decisões do G9 com confiança ou margem abaixo do limiar: nada inicia.",
            )?,
            g9_fallback: registry.counter(
                "poph_fluxos_g9_fallback_total",
                "Decisões do G9 que caíram no fallback do LLM porque o Jev falhou.",
            )?,
            pending_effects: registry.counter(
                "poph_fluxos_efeitos_pendentes_total",
                "Efeitos de fluxo que entraram no estado pendente.",
            )?,
            sent_effects: registry.counter(
                "poph_fluxos_efeitos_enviados_total",
                "Efeitos de fluxo que entraram no estado enviado.",
            )?,
            failed_effects: registry.counter(
                "poph_fluxos_efeitos_falhas_total",
                "Efeitos de fluxo que entraram no estado falhou, depois do limite de tentativas.",
            )?,
            rule_errors: registry.counter(
                "poph_fluxos_erros_regra_total",
                "Avaliações de regra de fluxo que falharam.",
            )?,
            last_cache: Arc::new(Mutex::new(CacheMetrics::default())),
        })
    }

    pub fn execution_metrics(&self) -> poph_store::metrics::ExecutionMetrics {
        poph_store::metrics::ExecutionMetrics {
            started: self.started_executions.clone(),
            completed: self.completed_executions.clone(),
            abandoned: self.abandoned_executions.clone(),
            failures: self.failed_executions.clone(),
            rule_errors: self.rule_errors.clone(),
        }
    }

    pub fn sync_cache(&self, current: CacheMetrics) {
        let mut last = self
            .last_cache
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let previous = *last;
        *last = current;
        self.cache_hits
            .increment(delta(current.hits, previous.hits));
        self.cache_errors
            .increment(delta(current.misses, previous.misses));
        self.cache_loads
            .increment(delta(current.loads, previous.loads));
        self.cache_waits
            .increment(delta(current.waits, previous.waits));
        self.evictions
            .increment(delta(current.evictions, previous.evictions));
        self.protected_in_use
            .increment(delta(current.in_use_spared, previous.in_use_spared));
        self.uncached_revisions
            .increment(delta(current.not_cached, previous.not_cached));
        self.cached_revisions
            .set(i64::try_from(current.entries).unwrap_or(i64::MAX));
        self.revisions_in_use
            .set(i64::try_from(current.in_use).unwrap_or(i64::MAX));
        self.cache_bytes
            .set(i64::try_from(current.bytes).unwrap_or(i64::MAX));
    }
}

fn delta(current: u64, previous: u64) -> u64 {
    current.saturating_sub(previous)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_duplicate_registration() {
        let registry = Registry::new();
        FlowMetrics::register(&registry).expect("primeiro registro");
        assert!(FlowMetrics::register(&registry).is_err());
    }

    #[test]
    fn each_series_appears_once_with_module_prefix() {
        let registry = Registry::new();
        let _ = FlowMetrics::register(&registry).expect("registra");
        let text = registry.render();
        let names: Vec<&str> = text
            .lines()
            .filter_map(|line| line.strip_prefix("# TYPE "))
            .filter_map(|rest| rest.split(' ').next())
            .collect();
        assert!(!names.is_empty());
        for name in &names {
            assert!(
                name.starts_with("poph_fluxos_"),
                "{name} sem o prefixo do módulo"
            );
            assert_eq!(
                text.matches(&format!("# HELP {name} ")).count(),
                1,
                "{name} deve aparecer uma vez"
            );
        }
        assert!(!text.contains('{'), "séries sem rótulos: {text}");
    }

    #[test]
    fn cache_deltas_update_counters() {
        let registry = Registry::new();
        let metrics = FlowMetrics::register(&registry).expect("registra");
        metrics.sync_cache(CacheMetrics {
            hits: 3,
            loads: 2,
            misses: 2,
            entries: 2,
            bytes: 4096,
            ..CacheMetrics::default()
        });
        metrics.sync_cache(CacheMetrics {
            hits: 5,
            loads: 2,
            misses: 2,
            evictions: 1,
            entries: 1,
            bytes: 2048,
            ..CacheMetrics::default()
        });
        assert_eq!(metrics.cache_hits.value(), 5);
        assert_eq!(metrics.cache_loads.value(), 2);
        assert_eq!(metrics.evictions.value(), 1);
        assert_eq!(metrics.cached_revisions.value(), 1);
        assert_eq!(metrics.cache_bytes.value(), 2048);
        let text = registry.render();
        assert!(
            text.contains("poph_fluxos_cache_acertos_total 5\n"),
            "{text}"
        );
        assert!(text.contains("poph_fluxos_expulsoes_total 1\n"), "{text}");
    }

    #[test]
    fn reading_twice_does_not_double_count() {
        let registry = Registry::new();
        let metrics = FlowMetrics::register(&registry).expect("registra");
        let reading = CacheMetrics {
            hits: 7,
            ..CacheMetrics::default()
        };
        metrics.sync_cache(reading);
        metrics.sync_cache(reading);
        metrics.sync_cache(reading);
        assert_eq!(metrics.cache_hits.value(), 7);
    }
}
