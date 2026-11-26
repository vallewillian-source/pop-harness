//! Erro da API no formato do PRD §8.1: `{code, message, details}`.
//!
//! Cada variante tem um código estável (o Console e os scripts casam pelo `code`, nunca pela
//! mensagem). A mensagem é sempre segura: não carrega SQL, caminho interno nem segredo.

use axum::Json;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use poph_store::StoreError;
use poph_types::DomainError;
use serde::Serialize;
use serde_json::{Value, json};
use utoipa::ToSchema;

/// Corpo de erro de qualquer rota.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RespostaErro {
    /// Código estável (`unauthorized`, `not_found`, `version_conflict`, ...).
    pub code: &'static str,
    /// Mensagem legível, em português.
    pub message: String,
    /// Contexto adicional (campo, versão esperada e atual, org). Pode ser `null`.
    pub details: Value,
}

/// Erro de uma rota: status HTTP + corpo padronizado.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    details: Value,
}

impl ApiError {
    /// Monta o erro com status e código.
    pub fn novo(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            details: Value::Null,
        }
    }

    /// Acrescenta detalhes (campo, versão, org).
    pub fn com_detalhes(mut self, details: Value) -> Self {
        self.details = details;
        self
    }

    /// 400 — o pedido não faz sentido (JSON inválido, campo mal preenchido).
    pub fn invalido(message: impl Into<String>) -> Self {
        Self::novo(StatusCode::BAD_REQUEST, "invalid_request", message)
    }

    /// 400 — id que não é UUID v7 ou cursor ilegível.
    pub fn id_invalido(campo: &'static str, valor: &str, motivo: impl Into<String>) -> Self {
        Self::novo(
            StatusCode::BAD_REQUEST,
            "invalid_id",
            format!("`{valor}` não é um id válido de {campo}"),
        )
        .com_detalhes(json!({ "field": campo, "reason": motivo.into() }))
    }

    /// 400 — campo recusado pela validação.
    ///
    /// O nome do campo vem do corpo da requisição ou do esquema do plugin (ex.:
    /// `config_json.api_key`), por isso é `String` e não `&'static str`.
    pub fn validacao(campo: impl Into<String>, motivo: impl Into<String>) -> Self {
        let campo = campo.into();
        Self::novo(
            StatusCode::BAD_REQUEST,
            "validation_error",
            format!("campo `{campo}` inválido"),
        )
        .com_detalhes(json!({ "field": campo, "reason": motivo.into() }))
    }

    /// 401 — token ausente, malformado, desconhecido ou revogado.
    pub fn nao_autorizado() -> Self {
        Self::novo(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "informe um token válido em `Authorization: Bearer <token>`",
        )
    }

    /// 403 — token de org tentando alcançar outra org, ou ação só de token global.
    pub fn proibido(detalhe: &str) -> Self {
        Self::novo(StatusCode::FORBIDDEN, "forbidden", detalhe)
    }

    /// 404 — registro inexistente (ou fora da org do token).
    pub fn nao_encontrado(entidade: &str, id: &str) -> Self {
        Self::novo(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("{entidade} `{id}` não encontrado"),
        )
    }

    /// 409 — conflito de negócio (ex.: excluir org com agentes).
    pub fn conflito(message: impl Into<String>) -> Self {
        Self::novo(StatusCode::CONFLICT, "conflict", message)
    }

    /// 412 — `If-Match` desatualizado.
    pub fn conflito_de_versao(entidade: &str, id: &str, esperada: u64, atual: u64) -> Self {
        Self::novo(
            StatusCode::PRECONDITION_FAILED,
            "version_conflict",
            format!("{entidade} `{id}` mudou desde a leitura"),
        )
        .com_detalhes(json!({ "expected_version": esperada, "current_version": atual }))
    }

    /// 428 — escrita sem o cabeçalho `If-Match`.
    pub fn if_match_obrigatorio(descricao: &str) -> Self {
        Self::novo(
            StatusCode::PRECONDITION_REQUIRED,
            "precondition_required",
            format!("envie o cabeçalho `If-Match` com a versão de {descricao}"),
        )
    }

    /// 500 — falha inesperada (o detalhe vai para o log, não para a resposta).
    pub fn interno() -> Self {
        Self::novo(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "erro interno do servidor",
        )
    }

    /// Status HTTP do erro (usado nos testes e no log de acesso).
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// Código estável do erro.
    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let corpo = RespostaErro {
            code: self.code,
            message: self.message,
            details: self.details,
        };
        let mut resposta = (self.status, Json(corpo)).into_response();
        // `WWW-Authenticate` só faz sentido no 401; nas outras respostas ele confundiria o cliente.
        if self.status == StatusCode::UNAUTHORIZED {
            resposta.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                header::HeaderValue::from_static("Bearer"),
            );
        }
        resposta
    }
}

impl From<StoreError> for ApiError {
    fn from(erro: StoreError) -> Self {
        match &erro {
            StoreError::Domain(dominio) => Self::do_dominio(dominio),
            outro => {
                tracing::error!(erro = %outro, "falha no armazenamento ao atender a API");
                Self::interno()
            }
        }
    }
}

impl ApiError {
    fn do_dominio(erro: &DomainError) -> Self {
        match erro {
            DomainError::InvalidId {
                entity: entidade,
                value,
                reason,
            } => Self::id_invalido(entidade, value, reason.clone()),
            DomainError::NotFound {
                entity: entidade,
                id,
            } => Self::nao_encontrado(entidade, id),
            DomainError::VersionConflict {
                entity: entidade,
                id,
                expected: esperada,
                current: atual,
            } => Self::conflito_de_versao(entidade, id, *esperada, *atual),
            DomainError::Validation { field, reason } => Self::validacao(*field, reason.clone()),
            DomainError::Conflict { reason, .. } => Self::conflito(reason.clone()),
            DomainError::AgentInactive { id, status } => {
                Self::conflito(format!("agente `{id}` está {status}: não abre turno novo"))
            }
            // Erro de domínio sem braço próprio: o código e o status vêm do próprio erro, que é
            // onde estão declarados (`DomainError::codigo_api`/`status_http`). Antes da FX-07 todo
            // caso novo caía em 400 `invalid_request`, o que mascarava, por exemplo, o 409
            // `conversa_encerrada` do `reopen`.
            outro => Self::do_codigo_do_dominio(outro),
        }
    }

    /// Monta o erro a partir de [`DomainError::codigo_api`] e [`DomainError::status_http`].
    fn do_codigo_do_dominio(erro: &DomainError) -> Self {
        let status = StatusCode::from_u16(erro.http_status()).unwrap_or(StatusCode::BAD_REQUEST);
        Self::novo(status, erro.api_code(), erro.to_string())
    }
}

/// Atalho usado pelos handlers que recebem um id pela URL.
pub fn id_da_url<T>(texto: &str) -> Result<T, ApiError>
where
    T: std::str::FromStr<Err = DomainError>,
    T: IdNomeavel,
{
    texto
        .parse::<T>()
        .map_err(|erro| ApiError::id_invalido(T::ENTIDADE, texto, erro.to_string()))
}

/// Nome da entidade de cada id, para a mensagem de id inválido.
pub trait IdNomeavel {
    /// Nome da entidade (`org`, `token`, ...).
    const ENTIDADE: &'static str;
}

macro_rules! nomeavel {
    ($tipo:ty, $nome:literal) => {
        impl IdNomeavel for $tipo {
            const ENTIDADE: &'static str = $nome;
        }
    };
}

nomeavel!(poph_types::OrgId, "org");
nomeavel!(poph_types::AdminTokenId, "token");
nomeavel!(poph_types::ProductCategoryId, "categoria de produto");
nomeavel!(poph_types::DepartmentId, "departamento");
nomeavel!(poph_types::AgentId, "agente");
nomeavel!(poph_types::FlowId, "fluxo");
nomeavel!(poph_types::FileId, "arquivo");

nomeavel!(poph_types::ConversationId, "conversa");
nomeavel!(poph_types::RecordId, "ficha");
nomeavel!(poph_types::MessageId, "mensagem");
nomeavel!(poph_types::AttachmentId, "anexo");
nomeavel!(poph_types::ProductId, "produto");

nomeavel!(poph_types::IntegrationId, "integração");
