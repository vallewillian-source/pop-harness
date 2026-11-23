//! Métricas `poph_fluxos_*` do módulo de Fluxos e Fichas (FX-26), no registro do processo (`/metrics`).
//!
//! Declaradas só com `[fluxos] ativo = true` ([`crate::Estado::metricas_fluxos`]). Sem rótulos: o
//! registro ainda não os tem, então cada resultado tem o seu contador. Nenhuma série leva id de Ficha,
//! telefone ou texto de cliente (cardinalidade e privacidade).
//!
//! Quem produz cada número:
//!
//! - `poph_fluxos_cache_*`, `poph_fluxos_expulsoes_total`, `poph_fluxos_revisoes_*` e
//!   `poph_fluxos_cache_bytes`: o cache de revisões (FX-19), lido a cada `/metrics`. Como o
//!   [`poph_fluxos::Metricas`] é cumulativo, cada leitura soma só a diferença desde a anterior.
//! - `poph_fluxos_execucoes_*`, `poph_fluxos_g9_*`, `poph_fluxos_efeitos_*` e `poph_fluxos_erros_regra_total`:
//!   contadores que quem executa o fluxo (FX-20 em diante, e o gate G9 na FX-22) vai incrementar. Até lá
//!   ficam em zero, e isso é esperado.

use std::sync::{Arc, Mutex};

use poph_fluxos::Metricas as MetricasCache;
use poph_types::{Contador, DomainResult, Medidor, Registro};

/// Os números do módulo de fluxos. Todos os campos são handles baratos de clonar.
#[derive(Debug, Clone)]
pub struct MetricasFluxos {
    /// Consultas servidas por uma revisão já carregada (`poph_fluxos_cache_acertos_total`).
    pub cache_acertos: Contador,
    /// Consultas que não acharam a revisão pronta (`poph_fluxos_cache_erros_total`).
    pub cache_erros: Contador,
    /// Revisões carregadas do script, uma compilação cada (`poph_fluxos_cache_cargas_total`).
    pub cache_cargas: Contador,
    /// Consultas que esperaram a carga de outra consulta da mesma revisão (`poph_fluxos_cache_esperas_total`).
    pub cache_esperas: Contador,
    /// Revisões expulsas do cache por falta de espaço (`poph_fluxos_expulsoes_total`).
    pub expulsoes: Contador,
    /// Revisões em uso que deixaram de ser expulsas (`poph_fluxos_cache_em_uso_poupadas_total`).
    pub cache_em_uso_poupadas: Contador,
    /// Revisões carregadas que não couberam no teto e não foram guardadas (`poph_fluxos_cache_nao_cacheadas_total`).
    pub cache_nao_cacheadas: Contador,
    /// Revisões guardadas no cache agora (`poph_fluxos_revisoes_em_cache`).
    pub revisoes_em_cache: Medidor,
    /// Revisões guardadas que alguém está usando agora (`poph_fluxos_revisoes_em_uso`).
    pub revisoes_em_uso: Medidor,
    /// Peso estimado das revisões guardadas, em bytes (`poph_fluxos_cache_bytes`).
    pub cache_bytes: Medidor,
    /// Execuções de fluxo iniciadas (`poph_fluxos_execucoes_iniciadas_total`).
    pub execucoes_iniciadas: Contador,
    /// Execuções que chegaram a `fim` (`poph_fluxos_execucoes_concluidas_total`).
    pub execucoes_concluidas: Contador,
    /// Execuções abandonadas (`poph_fluxos_execucoes_abandonadas_total`).
    pub execucoes_abandonadas: Contador,
    /// Execuções que pararam em falha: ciclo de decisões, teto de saltos, regra que não avalia
    /// (`poph_fluxos_execucoes_falhas_total`).
    pub execucoes_falhas: Contador,
    /// Decisões do G9 que iniciaram um fluxo (`poph_fluxos_g9_iniciou_total`).
    pub g9_iniciou: Contador,
    /// Decisões do G9 em que o Jev respondeu "nenhum" (`poph_fluxos_g9_nenhum_total`).
    pub g9_nenhum: Contador,
    /// Decisões do G9 com confiança ou margem abaixo do limiar (`poph_fluxos_g9_abaixo_do_limiar_total`).
    pub g9_abaixo_do_limiar: Contador,
    /// Decisões do G9 que caíram no fallback do LLM porque o Jev falhou (`poph_fluxos_g9_fallback_total`).
    pub g9_fallback: Contador,
    /// Efeitos que entraram no estado `pendente` (`poph_fluxos_efeitos_pendentes_total`).
    pub efeitos_pendentes: Contador,
    /// Efeitos que entraram no estado `enviado` (`poph_fluxos_efeitos_enviados_total`).
    pub efeitos_enviados: Contador,
    /// Efeitos que entraram no estado `falhou`, depois do limite de tentativas (`poph_fluxos_efeitos_falhas_total`).
    pub efeitos_falhas: Contador,
    /// Avaliações de regra que falharam (`poph_fluxos_erros_regra_total`).
    pub erros_regra: Contador,
    /// Última leitura do cache, para calcular a diferença na próxima.
    ultimo_cache: Arc<Mutex<MetricasCache>>,
}

impl MetricasFluxos {
    /// Declara o catálogo no registro dado. Repetir no mesmo registro é erro (o registro recusa).
    pub fn registrar(registro: &Registro) -> DomainResult<Self> {
        Ok(Self {
            cache_acertos: registro.contador(
                "poph_fluxos_cache_acertos_total",
                "Consultas de revisão de fluxo servidas pelo cache.",
            )?,
            cache_erros: registro.contador(
                "poph_fluxos_cache_erros_total",
                "Consultas de revisão de fluxo que não acharam a revisão pronta no cache.",
            )?,
            cache_cargas: registro.contador(
                "poph_fluxos_cache_cargas_total",
                "Revisões de fluxo compiladas do script (uma compilação cada).",
            )?,
            cache_esperas: registro.contador(
                "poph_fluxos_cache_esperas_total",
                "Consultas que esperaram a compilação de outra consulta da mesma revisão.",
            )?,
            expulsoes: registro.contador(
                "poph_fluxos_expulsoes_total",
                "Revisões de fluxo expulsas do cache por falta de espaço.",
            )?,
            cache_em_uso_poupadas: registro.contador(
                "poph_fluxos_cache_em_uso_poupadas_total",
                "Revisões em uso que não foram expulsas porque alguém ainda as segura.",
            )?,
            cache_nao_cacheadas: registro.contador(
                "poph_fluxos_cache_nao_cacheadas_total",
                "Revisões compiladas que não couberam no teto e não foram guardadas.",
            )?,
            revisoes_em_cache: registro.medidor(
                "poph_fluxos_revisoes_em_cache",
                "Revisões de fluxo guardadas no cache agora.",
            )?,
            revisoes_em_uso: registro.medidor(
                "poph_fluxos_revisoes_em_uso",
                "Revisões de fluxo guardadas que alguém está usando agora.",
            )?,
            cache_bytes: registro.medidor(
                "poph_fluxos_cache_bytes",
                "Peso estimado, em bytes, das revisões de fluxo guardadas no cache.",
            )?,
            execucoes_iniciadas: registro.contador(
                "poph_fluxos_execucoes_iniciadas_total",
                "Execuções de fluxo iniciadas.",
            )?,
            execucoes_concluidas: registro.contador(
                "poph_fluxos_execucoes_concluidas_total",
                "Execuções de fluxo que chegaram a um fim.",
            )?,
            execucoes_abandonadas: registro.contador(
                "poph_fluxos_execucoes_abandonadas_total",
                "Execuções de fluxo abandonadas.",
            )?,
            execucoes_falhas: registro.contador(
                "poph_fluxos_execucoes_falhas_total",
                "Execuções de fluxo que pararam em falha (decisão em ciclo, teto de saltos ou regra que não avalia).",
            )?,
            g9_iniciou: registro.contador(
                "poph_fluxos_g9_iniciou_total",
                "Decisões do G9 que iniciaram um fluxo.",
            )?,
            g9_nenhum: registro.contador(
                "poph_fluxos_g9_nenhum_total",
                "Decisões do G9 em que o Jev respondeu que nenhum fluxo serve.",
            )?,
            g9_abaixo_do_limiar: registro.contador(
                "poph_fluxos_g9_abaixo_do_limiar_total",
                "Decisões do G9 com confiança ou margem abaixo do limiar: nada inicia.",
            )?,
            g9_fallback: registro.contador(
                "poph_fluxos_g9_fallback_total",
                "Decisões do G9 que caíram no fallback do LLM porque o Jev falhou.",
            )?,
            efeitos_pendentes: registro.contador(
                "poph_fluxos_efeitos_pendentes_total",
                "Efeitos de fluxo que entraram no estado pendente.",
            )?,
            efeitos_enviados: registro.contador(
                "poph_fluxos_efeitos_enviados_total",
                "Efeitos de fluxo que entraram no estado enviado.",
            )?,
            efeitos_falhas: registro.contador(
                "poph_fluxos_efeitos_falhas_total",
                "Efeitos de fluxo que entraram no estado falhou, depois do limite de tentativas.",
            )?,
            erros_regra: registro.contador(
                "poph_fluxos_erros_regra_total",
                "Avaliações de regra de fluxo que falharam.",
            )?,
            ultimo_cache: Arc::new(Mutex::new(MetricasCache::default())),
        })
    }

    /// Leva as contas do cache de revisões para as séries.
    ///
    /// Os contadores recebem só a diferença desde a leitura anterior (o cache conta desde que
    /// nasceu; `limpar()` não zera essas contas). Os medidores recebem o valor de agora.
    pub fn sincronizar_cache(&self, atual: MetricasCache) {
        let mut ultimo = self
            .ultimo_cache
            .lock()
            .unwrap_or_else(|veneno| veneno.into_inner());
        let anterior = *ultimo;
        *ultimo = atual;
        self.cache_acertos
            .incrementar(diferenca(atual.acertos, anterior.acertos));
        self.cache_erros
            .incrementar(diferenca(atual.erros, anterior.erros));
        self.cache_cargas
            .incrementar(diferenca(atual.cargas, anterior.cargas));
        self.cache_esperas
            .incrementar(diferenca(atual.esperas, anterior.esperas));
        self.expulsoes
            .incrementar(diferenca(atual.expulsoes, anterior.expulsoes));
        self.cache_em_uso_poupadas
            .incrementar(diferenca(atual.em_uso_poupadas, anterior.em_uso_poupadas));
        self.cache_nao_cacheadas
            .incrementar(diferenca(atual.nao_cacheadas, anterior.nao_cacheadas));
        self.revisoes_em_cache
            .definir(i64::try_from(atual.entradas).unwrap_or(i64::MAX));
        self.revisoes_em_uso
            .definir(i64::try_from(atual.em_uso).unwrap_or(i64::MAX));
        self.cache_bytes
            .definir(i64::try_from(atual.bytes).unwrap_or(i64::MAX));
    }
}

/// Diferença entre duas leituras de um número que só cresce. Se algo recuou (não deveria), conta zero.
fn diferenca(atual: u64, anterior: u64) -> u64 {
    atual.saturating_sub(anterior)
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn catalogo_registra_uma_vez_e_recusa_repetir_no_mesmo_registro() {
        let registro = Registro::new();
        MetricasFluxos::registrar(&registro).expect("primeiro registro");
        assert!(MetricasFluxos::registrar(&registro).is_err());
    }

    #[test]
    fn toda_serie_aparece_uma_vez_com_o_prefixo_do_modulo() {
        let registro = Registro::new();
        let _ = MetricasFluxos::registrar(&registro).expect("registra");
        let texto = registro.renderizar();
        let nomes: Vec<&str> = texto
            .lines()
            .filter_map(|linha| linha.strip_prefix("# TYPE "))
            .filter_map(|resto| resto.split(' ').next())
            .collect();
        assert!(!nomes.is_empty());
        for nome in &nomes {
            assert!(
                nome.starts_with("poph_fluxos_"),
                "{nome} sem o prefixo do módulo"
            );
            assert_eq!(
                texto.matches(&format!("# HELP {nome} ")).count(),
                1,
                "{nome} deve aparecer uma vez"
            );
        }
        // Nenhuma série leva rótulo: o registro ainda não tem, e o catálogo não pode pedir id de Ficha.
        assert!(!texto.contains('{'), "séries sem rótulos: {texto}");
    }

    #[test]
    fn contas_do_cache_viram_contadores_pela_diferenca() {
        let registro = Registro::new();
        let metricas = MetricasFluxos::registrar(&registro).expect("registra");
        metricas.sincronizar_cache(MetricasCache {
            acertos: 3,
            cargas: 2,
            erros: 2,
            entradas: 2,
            bytes: 4096,
            ..MetricasCache::default()
        });
        // Segunda leitura: só a diferença entra, e o medidor reflete o agora.
        metricas.sincronizar_cache(MetricasCache {
            acertos: 5,
            cargas: 2,
            erros: 2,
            expulsoes: 1,
            entradas: 1,
            bytes: 2048,
            ..MetricasCache::default()
        });
        assert_eq!(metricas.cache_acertos.valor(), 5);
        assert_eq!(metricas.cache_cargas.valor(), 2);
        assert_eq!(metricas.expulsoes.valor(), 1);
        assert_eq!(metricas.revisoes_em_cache.valor(), 1);
        assert_eq!(metricas.cache_bytes.valor(), 2048);
        let texto = registro.renderizar();
        assert!(
            texto.contains("poph_fluxos_cache_acertos_total 5\n"),
            "{texto}"
        );
        assert!(texto.contains("poph_fluxos_expulsoes_total 1\n"), "{texto}");
    }

    #[test]
    fn leitura_repetida_nao_soma_de_novo() {
        let registro = Registro::new();
        let metricas = MetricasFluxos::registrar(&registro).expect("registra");
        let leitura = MetricasCache {
            acertos: 7,
            ..MetricasCache::default()
        };
        metricas.sincronizar_cache(leitura);
        metricas.sincronizar_cache(leitura);
        metricas.sincronizar_cache(leitura);
        assert_eq!(metricas.cache_acertos.valor(), 7);
    }
}
