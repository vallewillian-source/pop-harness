use std::sync::Arc;

use poph_types::{Counter, DomainResult, Gauge, Histogram, Registry};

#[derive(Debug, Clone)]
pub struct MetricsMatrix {
    pub cursor_age_seconds: Gauge,
    pub ready_rooms: Counter,
    pub mirrored_messages: Counter,
    pub mirrored_assets: Counter,
    pub responses_queued: Counter,
    pub responses_duplicated: Counter,
    pub responses_conversation_closed: Counter,
    pub rooms_unknown: Counter,
    pub asset_notices: Counter,
    pub transient_errors: Counter,
    pub definitive_errors: Counter,
    pub responses_429: Counter,
    pub attempts: Counter,
    pub isolated_messages: Counter,
    pub redacted_assets: Counter,
    pub assets_deleted: Counter,
    pub assets_expiry_deferred: Counter,
    pub assets_pending_expiry: Gauge,
    pub latency_synapse_seconds: Histogram,
}

impl MetricsMatrix {
    pub fn register(registry: &Registry) -> DomainResult<Self> {
        Ok(Self {
            cursor_age_seconds: registry.gauge(
                "poph_matrix_atraso_cursor_segundos",
                "Idade, em segundos, da mensagem mais antiga ainda não espelhada para o Matrix.",
            )?,
            ready_rooms: registry.counter(
                "poph_matrix_salas_prontas_total",
                "Salas do Matrix que ficaram prontas nesta passagem (criadas ou concluídas).",
            )?,
            mirrored_messages: registry.counter(
                "poph_matrix_mensagens_espelhadas_total",
                "Mensagens do Harness entregues ao Synapse pelo espelhamento.",
            )?,
            mirrored_assets: registry.counter(
                "poph_matrix_midias_espelhadas_total",
                "Mídia do cliente entregue à sala pelo espelhamento (imagem ou arquivo).",
            )?,
            responses_queued: registry.counter(
                "poph_matrix_respostas_enfileiradas_total",
                "Respostas do atendente gravadas para envio ao cliente.",
            )?,
            responses_duplicated: registry.counter(
                "poph_matrix_respostas_duplicadas_total",
                "Reenvios do Synapse de uma resposta do atendente que já estava gravada.",
            )?,
            responses_conversation_closed: registry.counter(
                "poph_matrix_respostas_conversa_encerrada_total",
                "Respostas do atendente cuja conversa estava encerrada: não enviadas ao cliente.",
            )?,
            rooms_unknown: registry.counter(
                "poph_matrix_salas_desconhecidas_total",
                "Eventos de uma sala que este Harness não reconhece: nada foi gravado.",
            )?,
            asset_notices: registry.counter(
                "poph_matrix_avisos_de_midia_total",
                "Avisos de mídia do atendente escritos na sala (envio de arquivo fora da fase 1).",
            )?,
            transient_errors: registry.counter(
                "poph_matrix_erros_transitorios_total",
                "Chamadas ao Synapse que falharam de forma transitória.",
            )?,
            definitive_errors: registry.counter(
                "poph_matrix_erros_definitivos_total",
                "Chamadas ao Synapse que falharam de forma definitiva.",
            )?,
            responses_429: registry.counter(
                "poph_matrix_respostas_429_total",
                "Respostas 429 (limite de taxa) do Synapse.",
            )?,
            attempts: registry.counter(
                "poph_matrix_tentativas_total",
                "Chamadas ao Synapse feitas pelo espelhamento, inclusive as que falharam.",
            )?,
            isolated_messages: registry.counter(
                "poph_matrix_mensagens_isoladas_total",
                "Mensagens isoladas em matrix_falhas depois de falhas definitivas ou formato inválido.",
            )?,
            redacted_assets: registry.counter(
                "poph_matrix_midias_redigidas_total",
                "Mídias cujo evento foi redigido na sala depois do prazo de retenção.",
            )?,
            assets_deleted: registry.counter(
                "poph_matrix_midias_apagadas_total",
                "Mídias cujo arquivo foi apagado do repositório do Synapse depois do prazo.",
            )?,
            assets_expiry_deferred: registry.counter(
                "poph_matrix_midias_expiracao_adiadas_total",
                "Mídias que a expiração adiou por falha e vai tentar de novo mais tarde.",
            )?,
            assets_pending_expiry: registry.gauge(
                "poph_matrix_midias_pendentes_expiracao",
                "Mídias vencidas que ainda têm etapa pendente (arquivo no Synapse ou evento sem redação).",
            )?,
            latency_synapse_seconds: registry.histogram(
                "poph_matrix_latencia_synapse_segundos",
                "Duração de cada operação do espelho contra o Synapse, em segundos.",
                &[0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0],
            )?,
        })
    }

    pub fn unregistered() -> Arc<Self> {
        let registry = Registry::new();
        let metrics = Self::register(&registry).unwrap_or_else(|error| {
            unreachable!("o catálogo do módulo Matrix registra em registro vazio: {error}")
        });
        Arc::new(metrics)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_registers_once_and_refuses_repeat_in_same_registry() {
        let registry = Registry::new();
        MetricsMatrix::register(&registry).expect("primeiro registro");
        assert!(MetricsMatrix::register(&registry).is_err());
    }

    #[test]
    fn names_follow_prefix_and_output_has_each_once() {
        let registry = Registry::new();
        let metrics = MetricsMatrix::register(&registry).expect("registra");
        metrics.mirrored_messages.increment(2);
        let text = registry.render();
        for name in [
            "poph_matrix_atraso_cursor_segundos",
            "poph_matrix_mensagens_espelhadas_total",
            "poph_matrix_latencia_synapse_segundos",
        ] {
            assert_eq!(
                text.matches(&format!("# HELP {name} ")).count(),
                1,
                "{name} deve aparecer uma vez"
            );
        }
        assert!(text.contains("poph_matrix_mensagens_espelhadas_total 2\n"));
    }
}
