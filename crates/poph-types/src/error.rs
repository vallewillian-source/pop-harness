//! Erro raiz do domínio. Cada variante carrega o contexto (entidade, id, campo)
//! para que a mensagem diga o que falhou sem depender de quem a captura.

use thiserror::Error;

/// Resultado padrão dos crates do Harness.
pub type DomainResult<T> = Result<T, DomainError>;

/// Erros do domínio. Não usa `anyhow`: cada caller decide como traduzir (API, CLI, log).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum DomainError {
    /// Texto ou UUID que não representa um id válido da entidade (ex.: não é UUID v7).
    #[error("id de {entidade} inválido ({valor}): {motivo}")]
    IdInvalido {
        entidade: &'static str,
        valor: String,
        motivo: String,
    },

    /// Registro não existe (ou não existe dentro da org consultada).
    #[error("{entidade} {id} não encontrado")]
    NaoEncontrado { entidade: &'static str, id: String },

    /// Escrita com `If-Match` desatualizado: a versão gravada mudou desde a leitura.
    #[error("conflito de versão em {entidade} {id}: esperada {esperada}, atual {atual}")]
    ConflitoDeVersao {
        entidade: &'static str,
        id: String,
        esperada: u64,
        atual: u64,
    },

    /// Valor de campo recusado pela validação do domínio.
    #[error("campo {campo} inválido: {motivo}")]
    Validacao { campo: &'static str, motivo: String },

    /// Operação recusada porque o registro tem conteúdo que impede (ex.: excluir org com agentes).
    ///
    /// Diferente de [`DomainError::Validacao`] (campo mal preenchido): aqui o pedido está bem
    /// formado e mesmo assim não pode ser atendido sem uma confirmação explícita.
    #[error("{entidade} {id} não pode ser excluído: {motivo}")]
    Conflito {
        entidade: &'static str,
        id: String,
        motivo: String,
    },

    /// Agente fora de `active` tentou abrir turno (decisão D-05: inativo quer dizer inativo).
    #[error("agente {id} está {status}: não abre turno novo")]
    AgenteInativo { id: String, status: String },

    /// Ficha encerrada tentou receber evento, execução ou ação (PRD L5).
    ///
    /// A saída não é reabrir a Ficha: a próxima mensagem do cliente abre conversa e Ficha novas.
    #[error("ficha {id} está encerrada: mensagem nova abre outra ficha")]
    FichaFechada { id: String },

    /// A Ficha já tem execução de fluxo ativa (PRD L6: uma por vez).
    #[error("ficha {ficha_id} já tem a execução {execucao_id} ativa")]
    ExecucaoJaAtiva {
        ficha_id: String,
        execucao_id: String,
    },

    /// Fluxo não liberado ao agente (PRD L8: fluxo novo começa negado para todos).
    #[error("fluxo {fluxo_id} não está liberado para o agente {agent_id}")]
    FluxoNaoLiberado { fluxo_id: String, agent_id: String },

    /// Ação que não existe na etapa atual, ou que o ator não pode executar.
    ///
    /// O motivo enumera as ações válidas: o agente lê a mensagem e corrige a chamada.
    #[error("ação `{acao}` recusada: {motivo}")]
    AcaoInvalida { acao: String, motivo: String },

    /// Campos de uma etapa recusados: a ação não grava nada pela metade (PRD §6.4).
    ///
    /// Cada item de `erros` diz campo e motivo ("periodo: opção `tarde` não existe").
    #[error("campos recusados na ficha {ficha_id}: {}", .erros.join("; "))]
    CamposInvalidos {
        ficha_id: String,
        erros: Vec<String>,
    },

    /// `reopen` de conversa `closed` (PRD L13): conversa encerrada não volta, só uma mensagem nova
    /// do cliente abre outra (com Ficha nova). Conversa `transferred` continua reabrindo.
    #[error("conversa {id} está encerrada: a próxima mensagem do cliente abre outra")]
    ConversaEncerrada { id: String },

    /// A IR gravada da revisão diverge da recompilação do texto (`FLXR06`).
    ///
    /// O motor **falha fechado**: não escolhe uma das duas versões. Quem conserta é quem publica
    /// uma revisão nova a partir do texto (FX-10, `poph_fluxos::carregar`).
    #[error("a estrutura gravada do fluxo {fluxo} não confere com o texto: {motivo}")]
    IrDivergente { fluxo: String, motivo: String },

    /// Regra de fluxo que não pôde ser avaliada por defeito do **autor** do fluxo: `funcao` sem
    /// `def` correspondente, `def` que devolveu algo que não é `True`/`False`, `def` que estourou
    /// (chave ausente, divisão por zero), comparação com literal que não é número, campo escrito
    /// fora do formato `etapa.campo`.
    ///
    /// É a falha de execução `FLXR01`/`FLXR02` (`FLUXOS_LINGUAGEM.md` §10.3): a execução do fluxo
    /// falha, a Ficha continua aberta e nada disso derruba o processo nem o turno.
    #[error("regra inválida: {motivo}")]
    RegraInvalida { motivo: String },

    /// Regra passou de um limite de avaliação: ticks, heap, profundidade de chamada ou prazo.
    ///
    /// É a falha de execução `FLXR03`: mesmo desfecho do [`DomainError::RegraInvalida`] (execução
    /// `falha`, Ficha aberta), mas a causa é custo, não defeito.
    #[error("limite da regra estourado: {motivo}")]
    RegraLimite { motivo: String },

    /// Agente passaria do teto de fluxos liberados (PRD §15: 20 por agente, para o G9 do Jev ter
    /// opções finitas). A permissão que estoura é recusada inteira; quem quer outra revoga antes.
    #[error(
        "agente {agent_id}: {pedidos} fluxos liberados passam do limite de {limite}; revogue algum antes"
    )]
    TetoDeFluxosPorAgente {
        agent_id: String,
        limite: u32,
        pedidos: u32,
    },
}

impl DomainError {
    /// Código estável de API (o `code` do corpo de erro, PRD §8.1).
    ///
    /// O Console e os scripts casam por este texto, nunca pela mensagem. Os nomes seguem a
    /// convenção do `docs/API.md`: genéricos em inglês (`validation_error`), conflitos próprios do
    /// domínio em português (`ficha_encerrada`).
    pub fn codigo_api(&self) -> &'static str {
        match self {
            DomainError::IdInvalido { .. } => "invalid_id",
            DomainError::NaoEncontrado { .. } => "not_found",
            DomainError::ConflitoDeVersao { .. } => "version_conflict",
            DomainError::Validacao { .. } => "validation_error",
            DomainError::Conflito { .. } => "conflict",
            DomainError::AgenteInativo { .. } => "conflict",
            DomainError::FichaFechada { .. } => "ficha_encerrada",
            DomainError::ExecucaoJaAtiva { .. } => "execucao_ativa",
            DomainError::FluxoNaoLiberado { .. } => "fluxo_nao_liberado",
            DomainError::AcaoInvalida { .. } => "acao_invalida",
            DomainError::CamposInvalidos { .. } => "campos_invalidos",
            DomainError::ConversaEncerrada { .. } => "conversa_encerrada",
            DomainError::IrDivergente { .. } => "ir_divergente",
            DomainError::TetoDeFluxosPorAgente { .. } => "teto_de_fluxos_por_agente",
            DomainError::RegraInvalida { .. } => "regra_invalida",
            DomainError::RegraLimite { .. } => "regra_limite",
        }
    }

    /// Status HTTP que acompanha o [`codigo_api`](DomainError::codigo_api).
    ///
    /// É contrato, não implementação: a API (`poph-server`) traduz este número no `ApiError`. Fica
    /// aqui para que o código e o status não divirjam entre módulos.
    pub fn status_http(&self) -> u16 {
        match self {
            DomainError::IdInvalido { .. } | DomainError::Validacao { .. } => 400,
            DomainError::NaoEncontrado { .. } => 404,
            DomainError::FluxoNaoLiberado { .. } => 403,
            DomainError::ConflitoDeVersao { .. } => 412,
            DomainError::Conflito { .. }
            | DomainError::AgenteInativo { .. }
            | DomainError::FichaFechada { .. }
            | DomainError::ExecucaoJaAtiva { .. }
            | DomainError::AcaoInvalida { .. }
            | DomainError::ConversaEncerrada { .. }
            | DomainError::IrDivergente { .. }
            | DomainError::TetoDeFluxosPorAgente { .. } => 409,
            DomainError::CamposInvalidos { .. } => 400,
            // Defeito do fluxo publicado ou custo da regra: nada disso é culpa do pedido do
            // cliente, e a saída é corrigir/republicar o fluxo, então o status é o mesmo de
            // conteúdo que não fecha (400), como os campos recusados.
            DomainError::RegraInvalida { .. } | DomainError::RegraLimite { .. } => 400,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Os erros novos do módulo de Fluxos e Fichas com o código de API e o status combinados.
    fn erros_do_modulo() -> Vec<(DomainError, &'static str, u16)> {
        vec![
            (
                DomainError::FichaFechada {
                    id: "ficha-1".to_owned(),
                },
                "ficha_encerrada",
                409,
            ),
            (
                DomainError::ExecucaoJaAtiva {
                    ficha_id: "ficha-1".to_owned(),
                    execucao_id: "exec-1".to_owned(),
                },
                "execucao_ativa",
                409,
            ),
            (
                DomainError::FluxoNaoLiberado {
                    fluxo_id: "fluxo-1".to_owned(),
                    agent_id: "agente-1".to_owned(),
                },
                "fluxo_nao_liberado",
                403,
            ),
            (
                DomainError::AcaoInvalida {
                    acao: "confirmar".to_owned(),
                    motivo: "só com `periodo` preenchido".to_owned(),
                },
                "acao_invalida",
                409,
            ),
            (
                DomainError::CamposInvalidos {
                    ficha_id: "ficha-1".to_owned(),
                    erros: vec!["periodo: opção `tarde` não existe".to_owned()],
                },
                "campos_invalidos",
                400,
            ),
            (
                DomainError::ConversaEncerrada {
                    id: "conversa-1".to_owned(),
                },
                "conversa_encerrada",
                409,
            ),
            (
                DomainError::TetoDeFluxosPorAgente {
                    agent_id: "agente-1".to_owned(),
                    limite: 20,
                    pedidos: 21,
                },
                "teto_de_fluxos_por_agente",
                409,
            ),
            (
                DomainError::IrDivergente {
                    fluxo: "agendar_visita".to_owned(),
                    motivo: "as etapas diferem da recompilação do texto".to_owned(),
                },
                "ir_divergente",
                409,
            ),
            (
                DomainError::RegraInvalida {
                    motivo: "a função `problema_grave` devolveu algo que não é `True`/`False`"
                        .to_owned(),
                },
                "regra_invalida",
                400,
            ),
            (
                DomainError::RegraLimite {
                    motivo: "a função `problema_grave` passou do prazo de 200 ms".to_owned(),
                },
                "regra_limite",
                400,
            ),
        ]
    }

    #[test]
    fn erros_do_modulo_tem_codigo_e_status_de_api() {
        for (erro, codigo, status) in erros_do_modulo() {
            assert_eq!(erro.codigo_api(), codigo, "código de {erro:?}");
            assert_eq!(erro.status_http(), status, "status de {erro:?}");
        }
    }

    #[test]
    fn codigos_de_api_dos_erros_novos_sao_distintos() {
        let mut codigos: Vec<&str> = erros_do_modulo()
            .iter()
            .map(|(erro, _, _)| erro.codigo_api())
            .collect();
        let total = codigos.len();
        codigos.sort_unstable();
        codigos.dedup();
        assert_eq!(
            codigos.len(),
            total,
            "dois erros novos com o mesmo código de API"
        );
    }

    #[test]
    fn codigos_antigos_seguem_o_docs_api() {
        // Os mesmos textos que `docs/API.md` publica: mudar aqui muda o contrato.
        let casos = [
            (
                DomainError::IdInvalido {
                    entidade: "agente",
                    valor: "x".to_owned(),
                    motivo: "não é UUID".to_owned(),
                },
                "invalid_id",
                400,
            ),
            (
                DomainError::NaoEncontrado {
                    entidade: "agente",
                    id: "x".to_owned(),
                },
                "not_found",
                404,
            ),
            (
                DomainError::ConflitoDeVersao {
                    entidade: "produto",
                    id: "x".to_owned(),
                    esperada: 1,
                    atual: 2,
                },
                "version_conflict",
                412,
            ),
            (
                DomainError::Validacao {
                    campo: "slug",
                    motivo: "vazio".to_owned(),
                },
                "validation_error",
                400,
            ),
            (
                DomainError::Conflito {
                    entidade: "categoria",
                    id: "x".to_owned(),
                    motivo: "tem produtos".to_owned(),
                },
                "conflict",
                409,
            ),
            (
                DomainError::AgenteInativo {
                    id: "x".to_owned(),
                    status: "paused".to_owned(),
                },
                "conflict",
                409,
            ),
        ];
        for (erro, codigo, status) in casos {
            assert_eq!(erro.codigo_api(), codigo, "código de {erro:?}");
            assert_eq!(erro.status_http(), status, "status de {erro:?}");
        }
    }

    #[test]
    fn campos_invalidos_enumera_todos_os_erros_na_mensagem() {
        let erro = DomainError::CamposInvalidos {
            ficha_id: "ficha-1".to_owned(),
            erros: vec![
                "periodo: opção `tarde` não existe".to_owned(),
                "cep: obrigatório".to_owned(),
            ],
        };
        let texto = erro.to_string();
        assert!(texto.contains("ficha-1"), "mensagem: {texto}");
        assert!(texto.contains("periodo"), "mensagem: {texto}");
        assert!(texto.contains("cep"), "mensagem: {texto}");
        assert!(texto.contains("; "), "mensagem sem separador: {texto}");
    }

    #[test]
    fn fluxo_nao_liberado_diz_qual_fluxo_e_qual_agente() {
        let erro = DomainError::FluxoNaoLiberado {
            fluxo_id: "fluxo-9".to_owned(),
            agent_id: "agente-3".to_owned(),
        };
        let texto = erro.to_string();
        assert!(texto.contains("fluxo-9"), "mensagem: {texto}");
        assert!(texto.contains("agente-3"), "mensagem: {texto}");
    }
}
