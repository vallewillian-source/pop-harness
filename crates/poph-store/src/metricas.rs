//! Contadores das execuções de fluxo (FX-40), no ponto em que `fluxo_execucoes` muda de estado.
//!
//! A semântica de cada número é a do catálogo de `/metrics` (`poph_fluxos_execucoes_*` e
//! `poph_fluxos_erros_regra_total`, declarados em `poph-server`). Eles moram **aqui** porque a
//! tabela é escrita por este crate e só aqui todas as portas se encontram: a abertura (turno e API),
//! a transição (agente e atendente), o fechamento da Ficha (`atendimento_encerrado`) e a exclusão do
//! fluxo (`fluxo_excluido`). Contar em cada chamador daria quatro contas para o mesmo fato, e a
//! mesma execução poderia entrar duas vezes.
//!
//! Quem declara as séries é o servidor (`[`crate`] do `poph-server`: `MetricasFluxos::registrar`),
//! que monta este catálogo com os mesmos handles e o publica com
//! [`crate::Store::publicar_metricas_de_execucao`]. Sem publicação (`poph migrate`, a CLI e os testes
//! que só usam o banco) os contadores não existem e nenhuma conta é feita: não há registro para onde
//! publicar.
//!
//! Os incrementos acontecem dentro do comando de escrita, antes do `COMMIT` do lote: se o commit
//! falhar depois, o contador conta uma operação que o banco desfez. É métrica de operação, não dado
//! do atendimento — o erro fica no log de quem chamou, e a contagem segue.

use poph_types::{Contador, StatusExecucao};

/// Os contadores do ciclo de vida da execução de fluxo (FX-40).
///
/// Todos são handles baratos de clonar (`Arc`): o mesmo catálogo pode ser passado ao
/// [`crate::Store`] e lido em qualquer thread.
#[derive(Debug, Clone)]
pub struct MetricasExecucao {
    /// Execuções abertas (`poph_fluxos_execucoes_iniciadas_total`).
    pub iniciadas: Contador,
    /// Execuções que chegaram a um `fim` (`poph_fluxos_execucoes_concluidas_total`).
    pub concluidas: Contador,
    /// Execuções abandonadas: pelo agente, pelo atendente, pelo fechamento da Ficha ou pela exclusão
    /// do fluxo (`poph_fluxos_execucoes_abandonadas_total`).
    pub abandonadas: Contador,
    /// Execuções que pararam em `falha` (`poph_fluxos_execucoes_falhas_total`).
    pub falhas: Contador,
    /// Avaliações de regra que falharam (`poph_fluxos_erros_regra_total`).
    pub erros_regra: Contador,
}

impl MetricasExecucao {
    /// Uma execução nasceu.
    pub(crate) fn abriu(&self, quantas: u64) {
        if quantas > 0 {
            self.iniciadas.incrementar(quantas);
        }
    }

    /// Uma execução mudou de situação: conta no contador da situação nova. `ativa` não conta (a
    /// execução continua em andamento) e nada é contado duas vezes, porque a situação só muda uma
    /// vez por desfecho.
    pub(crate) fn desfecho(&self, status: StatusExecucao, quantas: u64) {
        if quantas == 0 {
            return;
        }
        match status {
            StatusExecucao::Ativa => {}
            StatusExecucao::Concluida => self.concluidas.incrementar(quantas),
            StatusExecucao::Abandonada => self.abandonadas.incrementar(quantas),
            StatusExecucao::Falha => self.falhas.incrementar(quantas),
        }
    }

    /// Uma avaliação de regra falhou (a execução que a chamava vai para `falha`).
    pub(crate) fn regra_falhou(&self, quantas: u64) {
        if quantas > 0 {
            self.erros_regra.incrementar(quantas);
        }
    }
}
