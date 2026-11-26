use thiserror::Error;

pub type DomainResult<T> = Result<T, DomainError>;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum DomainError {
    #[error("id de {entity} inválido ({value}): {reason}")]
    InvalidId {
        entity: &'static str,
        value: String,
        reason: String,
    },

    #[error("{entity} {id} não encontrado")]
    NotFound { entity: &'static str, id: String },

    #[error("conflito de versão em {entity} {id}: esperada {expected}, atual {current}")]
    VersionConflict {
        entity: &'static str,
        id: String,
        expected: u64,
        current: u64,
    },

    #[error("campo {field} inválido: {reason}")]
    Validation { field: &'static str, reason: String },

    #[error("{entity} {id} não pode ser excluído: {reason}")]
    Conflict {
        entity: &'static str,
        id: String,
        reason: String,
    },

    #[error("agente {id} está {status}: não abre turno novo")]
    AgentInactive { id: String, status: String },

    #[error("ficha {id} está encerrada: mensagem nova abre outra ficha")]
    RecordClosed { id: String },

    #[error("ficha {ficha_id} já tem a execução {execucao_id} ativa")]
    ExecutionAlreadyActive {
        ficha_id: String,
        execucao_id: String,
    },

    #[error("fluxo {fluxo_id} não está liberado para o agente {agent_id}")]
    FlowNotAllowed { fluxo_id: String, agent_id: String },

    #[error("ação `{action}` recusada: {reason}")]
    InvalidAction { action: String, reason: String },

    #[error("campos recusados na ficha {ficha_id}: {}", .errors.join("; "))]
    InvalidFields {
        ficha_id: String,
        errors: Vec<String>,
    },

    #[error("campos recusados na etapa {step}: {}", .errors.join("; "))]
    InvalidStepFields { step: String, errors: Vec<String> },

    #[error("conversa {id} está encerrada: a próxima mensagem do cliente abre outra")]
    ConversationClosed { id: String },

    #[error("a estrutura gravada do fluxo {fluxo} não confere com o texto: {reason}")]
    IrDivergent { fluxo: String, reason: String },

    #[error("regra inválida: {reason}")]
    InvalidRule { reason: String },

    #[error("limite da regra estourado: {reason}")]
    RuleLimit { reason: String },

    #[error(
        "agente {agent_id}: {requested} fluxos liberados passam do limite de {limit}; revogue algum antes"
    )]
    FlowLimitPerAgent {
        agent_id: String,
        limit: u32,
        requested: u32,
    },
}

impl DomainError {
    pub fn api_code(&self) -> &'static str {
        match self {
            DomainError::InvalidId { .. } => "invalid_id",
            DomainError::NotFound { .. } => "not_found",
            DomainError::VersionConflict { .. } => "version_conflict",
            DomainError::Validation { .. } => "validation_error",
            DomainError::Conflict { .. } => "conflict",
            DomainError::AgentInactive { .. } => "conflict",
            DomainError::RecordClosed { .. } => "ficha_encerrada",
            DomainError::ExecutionAlreadyActive { .. } => "execucao_ativa",
            DomainError::FlowNotAllowed { .. } => "fluxo_nao_liberado",
            DomainError::InvalidAction { .. } => "acao_invalida",
            DomainError::InvalidFields { .. } => "campos_invalidos",
            DomainError::InvalidStepFields { .. } => "campos_invalidos",
            DomainError::ConversationClosed { .. } => "conversa_encerrada",
            DomainError::IrDivergent { .. } => "ir_divergente",
            DomainError::FlowLimitPerAgent { .. } => "teto_de_fluxos_por_agente",
            DomainError::InvalidRule { .. } => "regra_invalida",
            DomainError::RuleLimit { .. } => "regra_limite",
        }
    }

    pub fn http_status(&self) -> u16 {
        match self {
            DomainError::InvalidId { .. } | DomainError::Validation { .. } => 400,
            DomainError::NotFound { .. } => 404,
            DomainError::FlowNotAllowed { .. } => 403,
            DomainError::VersionConflict { .. } => 412,
            DomainError::Conflict { .. }
            | DomainError::AgentInactive { .. }
            | DomainError::RecordClosed { .. }
            | DomainError::ExecutionAlreadyActive { .. }
            | DomainError::InvalidAction { .. }
            | DomainError::ConversationClosed { .. }
            | DomainError::IrDivergent { .. }
            | DomainError::FlowLimitPerAgent { .. } => 409,
            DomainError::InvalidFields { .. } | DomainError::InvalidStepFields { .. } => 400,
            DomainError::InvalidRule { .. } | DomainError::RuleLimit { .. } => 400,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module_errors() -> Vec<(DomainError, &'static str, u16)> {
        vec![
            (
                DomainError::RecordClosed {
                    id: "ficha-1".to_owned(),
                },
                "ficha_encerrada",
                409,
            ),
            (
                DomainError::ExecutionAlreadyActive {
                    ficha_id: "ficha-1".to_owned(),
                    execucao_id: "exec-1".to_owned(),
                },
                "execucao_ativa",
                409,
            ),
            (
                DomainError::FlowNotAllowed {
                    fluxo_id: "fluxo-1".to_owned(),
                    agent_id: "agente-1".to_owned(),
                },
                "fluxo_nao_liberado",
                403,
            ),
            (
                DomainError::InvalidAction {
                    action: "confirmar".to_owned(),
                    reason: "só com `periodo` preenchido".to_owned(),
                },
                "acao_invalida",
                409,
            ),
            (
                DomainError::InvalidFields {
                    ficha_id: "ficha-1".to_owned(),
                    errors: vec!["periodo: opção `tarde` não existe".to_owned()],
                },
                "campos_invalidos",
                400,
            ),
            (
                DomainError::ConversationClosed {
                    id: "conversa-1".to_owned(),
                },
                "conversa_encerrada",
                409,
            ),
            (
                DomainError::FlowLimitPerAgent {
                    agent_id: "agente-1".to_owned(),
                    limit: 20,
                    requested: 21,
                },
                "teto_de_fluxos_por_agente",
                409,
            ),
            (
                DomainError::IrDivergent {
                    fluxo: "agendar_visita".to_owned(),
                    reason: "as etapas diferem da recompilação do texto".to_owned(),
                },
                "ir_divergente",
                409,
            ),
            (
                DomainError::InvalidRule {
                    reason: "a função `problema_grave` devolveu algo que não é `True`/`False`"
                        .to_owned(),
                },
                "regra_invalida",
                400,
            ),
            (
                DomainError::RuleLimit {
                    reason: "a função `problema_grave` passou do prazo de 200 ms".to_owned(),
                },
                "regra_limite",
                400,
            ),
        ]
    }

    #[test]
    fn module_errors_have_api_code_and_status() {
        for (error, code, status) in module_errors() {
            assert_eq!(error.api_code(), code, "código de {error:?}");
            assert_eq!(error.http_status(), status, "status de {error:?}");
        }
    }

    #[test]
    fn engine_rejected_fields_share_record_code() {
        let error = DomainError::InvalidStepFields {
            step: "coleta".to_owned(),
            errors: vec!["periodo: esperava uma das opções: `manhã`, `tarde`".to_owned()],
        };
        assert_eq!(error.api_code(), "campos_invalidos");
        assert_eq!(error.http_status(), 400);
        assert!(
            error.to_string().contains("coleta"),
            "a mensagem precisa dizer a etapa: {error}"
        );
    }

    #[test]
    fn new_error_api_codes_are_distinct() {
        let mut codes: Vec<&str> = module_errors()
            .iter()
            .map(|(error, _, _)| error.api_code())
            .collect();
        let total = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(
            codes.len(),
            total,
            "dois erros novos com o mesmo código de API"
        );
    }

    #[test]
    fn old_codes_follow_api_docs() {
        let cases = [
            (
                DomainError::InvalidId {
                    entity: "agente",
                    value: "x".to_owned(),
                    reason: "não é UUID".to_owned(),
                },
                "invalid_id",
                400,
            ),
            (
                DomainError::NotFound {
                    entity: "agente",
                    id: "x".to_owned(),
                },
                "not_found",
                404,
            ),
            (
                DomainError::VersionConflict {
                    entity: "produto",
                    id: "x".to_owned(),
                    expected: 1,
                    current: 2,
                },
                "version_conflict",
                412,
            ),
            (
                DomainError::Validation {
                    field: "slug",
                    reason: "vazio".to_owned(),
                },
                "validation_error",
                400,
            ),
            (
                DomainError::Conflict {
                    entity: "categoria",
                    id: "x".to_owned(),
                    reason: "tem produtos".to_owned(),
                },
                "conflict",
                409,
            ),
            (
                DomainError::AgentInactive {
                    id: "x".to_owned(),
                    status: "paused".to_owned(),
                },
                "conflict",
                409,
            ),
        ];
        for (error, code, status) in cases {
            assert_eq!(error.api_code(), code, "código de {error:?}");
            assert_eq!(error.http_status(), status, "status de {error:?}");
        }
    }

    #[test]
    fn invalid_fields_lists_every_error() {
        let error = DomainError::InvalidFields {
            ficha_id: "ficha-1".to_owned(),
            errors: vec![
                "periodo: opção `tarde` não existe".to_owned(),
                "cep: obrigatório".to_owned(),
            ],
        };
        let text = error.to_string();
        assert!(text.contains("ficha-1"), "mensagem: {text}");
        assert!(text.contains("periodo"), "mensagem: {text}");
        assert!(text.contains("cep"), "mensagem: {text}");
        assert!(text.contains("; "), "mensagem sem separador: {text}");
    }

    #[test]
    fn flow_not_allowed_names_flow_and_agent() {
        let error = DomainError::FlowNotAllowed {
            fluxo_id: "fluxo-9".to_owned(),
            agent_id: "agente-3".to_owned(),
        };
        let text = error.to_string();
        assert!(text.contains("fluxo-9"), "mensagem: {text}");
        assert!(text.contains("agente-3"), "mensagem: {text}");
    }
}
