//! Métricas `poph_matrix_*` do módulo Matrix (MX-26), no registro do processo (o `/metrics`).
//!
//! Declaradas **só quando o módulo é preparado**: [`crate::Estado::metricas_matrix`] registra na
//! primeira chamada e devolve o mesmo handle depois, então registrar duas vezes não duplica nada na
//! saída. Com o módulo desligado ninguém chama isso e nenhuma `poph_matrix_*` aparece no `/metrics`.
//!
//! O registro ainda não tem rótulos (ver `poph-types`), então não há séries por org: o atraso é o
//! pior entre as orgs ligadas, e cada classe de erro tem o seu contador.

use std::sync::Arc;

use poph_types::{Counter, DomainResult, Gauge, Histogram, Registry};

/// Os números do módulo Matrix. Todos os campos são handles baratos de clonar.
#[derive(Debug, Clone)]
pub struct MetricasMatrix {
    /// Idade, em segundos, da mensagem mais antiga que ainda não foi espelhada. `0` quando não há
    /// nada pendente. Sobe enquanto o Synapse não aceita, e volta a `0` quando o laço alcança o fim.
    pub atraso_cursor_segundos: Gauge,
    /// Salas que ficaram prontas nesta passagem (criadas agora ou concluídas depois de uma queda).
    pub salas_prontas: Counter,
    /// Mensagens do Harness entregues ao Synapse pelo espelhamento.
    pub mensagens_espelhadas: Counter,
    /// Mídia do cliente entregue à sala pelo espelhamento (MX-43): subida e evento.
    pub midias_espelhadas: Counter,
    /// Respostas do atendente gravadas para envio ao cliente (WhatsApp).
    pub respostas_enfileiradas: Counter,
    /// Reenvios do Synapse de uma resposta que já estava gravada (nada foi enviado de novo).
    pub respostas_duplicadas: Counter,
    /// Respostas do atendente cuja conversa já estava encerrada (não enviadas ao cliente).
    pub respostas_conversa_encerrada: Counter,
    /// Eventos recebidos de uma sala que este Harness não conhece (nada gravado).
    pub salas_desconhecidas: Counter,
    /// Avisos de mídia do atendente escritos na sala (o envio de arquivo não existe na fase 1).
    pub avisos_de_midia: Counter,
    /// Chamadas ao Synapse que falharam de forma transitória (rede, 5xx, prazo, 429).
    pub erros_transitorios: Counter,
    /// Chamadas ao Synapse que falharam de forma definitiva (4xx que não é 429).
    pub erros_definitivos: Counter,
    /// Respostas `429` do Synapse (limite de taxa), também contadas nos erros transitórios.
    pub respostas_429: Counter,
    /// Chamadas ao Synapse feitas pelo espelhamento para enviar ou provisionar, com ou sem sucesso.
    pub tentativas: Counter,
    /// Mensagens tiradas da fila do espelho e guardadas em `matrix_falhas` (não voltam sozinhas).
    pub mensagens_isoladas: Counter,
    /// Mídias cujo evento foi redigido na sala pela expiração (MX-46).
    pub midias_redigidas: Counter,
    /// Mídias cujo arquivo foi apagado do repositório do Synapse pela expiração (MX-46).
    pub midias_apagadas: Counter,
    /// Mídias que a expiração adiou por falha (voltam depois, com espera crescente).
    pub midias_expiracao_adiadas: Counter,
    /// Mídias vencidas que ainda não foram expiradas (arquivo no Synapse ou evento sem redação).
    pub midias_pendentes_expiracao: Gauge,
    /// Duração de cada operação do espelho contra o Synapse (envio de evento ou provisão de sala).
    pub latencia_synapse_segundos: Histogram,
}

impl MetricasMatrix {
    /// Declara o catálogo no registro dado. Repetir no mesmo registro é erro (o registro recusa).
    pub fn registrar(registro: &Registry) -> DomainResult<Self> {
        Ok(Self {
            atraso_cursor_segundos: registro.gauge(
                "poph_matrix_atraso_cursor_segundos",
                "Idade, em segundos, da mensagem mais antiga ainda não espelhada para o Matrix.",
            )?,
            salas_prontas: registro.counter(
                "poph_matrix_salas_prontas_total",
                "Salas do Matrix que ficaram prontas nesta passagem (criadas ou concluídas).",
            )?,
            mensagens_espelhadas: registro.counter(
                "poph_matrix_mensagens_espelhadas_total",
                "Mensagens do Harness entregues ao Synapse pelo espelhamento.",
            )?,
            midias_espelhadas: registro.counter(
                "poph_matrix_midias_espelhadas_total",
                "Mídia do cliente entregue à sala pelo espelhamento (imagem ou arquivo).",
            )?,
            respostas_enfileiradas: registro.counter(
                "poph_matrix_respostas_enfileiradas_total",
                "Respostas do atendente gravadas para envio ao cliente.",
            )?,
            respostas_duplicadas: registro.counter(
                "poph_matrix_respostas_duplicadas_total",
                "Reenvios do Synapse de uma resposta do atendente que já estava gravada.",
            )?,
            respostas_conversa_encerrada: registro.counter(
                "poph_matrix_respostas_conversa_encerrada_total",
                "Respostas do atendente cuja conversa estava encerrada: não enviadas ao cliente.",
            )?,
            salas_desconhecidas: registro.counter(
                "poph_matrix_salas_desconhecidas_total",
                "Eventos de uma sala que este Harness não reconhece: nada foi gravado.",
            )?,
            avisos_de_midia: registro.counter(
                "poph_matrix_avisos_de_midia_total",
                "Avisos de mídia do atendente escritos na sala (envio de arquivo fora da fase 1).",
            )?,
            erros_transitorios: registro.counter(
                "poph_matrix_erros_transitorios_total",
                "Chamadas ao Synapse que falharam de forma transitória.",
            )?,
            erros_definitivos: registro.counter(
                "poph_matrix_erros_definitivos_total",
                "Chamadas ao Synapse que falharam de forma definitiva.",
            )?,
            respostas_429: registro.counter(
                "poph_matrix_respostas_429_total",
                "Respostas 429 (limite de taxa) do Synapse.",
            )?,
            tentativas: registro.counter(
                "poph_matrix_tentativas_total",
                "Chamadas ao Synapse feitas pelo espelhamento, inclusive as que falharam.",
            )?,
            mensagens_isoladas: registro.counter(
                "poph_matrix_mensagens_isoladas_total",
                "Mensagens isoladas em matrix_falhas depois de falhas definitivas ou formato inválido.",
            )?,
            midias_redigidas: registro.counter(
                "poph_matrix_midias_redigidas_total",
                "Mídias cujo evento foi redigido na sala depois do prazo de retenção.",
            )?,
            midias_apagadas: registro.counter(
                "poph_matrix_midias_apagadas_total",
                "Mídias cujo arquivo foi apagado do repositório do Synapse depois do prazo.",
            )?,
            midias_expiracao_adiadas: registro.counter(
                "poph_matrix_midias_expiracao_adiadas_total",
                "Mídias que a expiração adiou por falha e vai tentar de novo mais tarde.",
            )?,
            midias_pendentes_expiracao: registro.gauge(
                "poph_matrix_midias_pendentes_expiracao",
                "Mídias vencidas que ainda têm etapa pendente (arquivo no Synapse ou evento sem redação).",
            )?,
            latencia_synapse_segundos: registro.histogram(
                "poph_matrix_latencia_synapse_segundos",
                "Duração de cada operação do espelho contra o Synapse, em segundos.",
                &[0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0],
            )?,
        })
    }

    /// Handles que não aparecem em nenhum `/metrics`: para o laço e a entrega quando ninguém
    /// injetou os números do processo (testes e montagens sem o módulo ligado).
    pub fn descartaveis() -> Arc<Self> {
        // Registro novo e vazio: o catálogo não tem nomes repetidos, então não pode falhar.
        let registro = Registry::new();
        let metricas = Self::registrar(&registro).unwrap_or_else(|error| {
            unreachable!("o catálogo do módulo Matrix registra em registro vazio: {error}")
        });
        Arc::new(metricas)
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn catalogo_registra_uma_vez_e_recusa_repetir_no_mesmo_registro() {
        let registro = Registry::new();
        MetricasMatrix::registrar(&registro).expect("primeiro registro");
        assert!(MetricasMatrix::registrar(&registro).is_err());
    }

    #[test]
    fn nomes_seguem_o_prefixo_e_a_saida_tem_cada_um_uma_vez() {
        let registro = Registry::new();
        let metricas = MetricasMatrix::registrar(&registro).expect("registra");
        metricas.mensagens_espelhadas.increment(2);
        let text = registro.render();
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
