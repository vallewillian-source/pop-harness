use axum::Json;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use poph_store::StoreError;
use poph_types::DomainError;
use serde::Serialize;
use serde_json::{Value, json};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = RespostaErro)]
#[doc = "Corpo de erro de qualquer rota."]
pub struct ErrorResponse {
    #[doc = "Código estável (`unauthorized`, `not_found`, `version_conflict`, ...)."]
    pub code: &'static str,
    #[doc = "Mensagem legível, em português."]
    pub message: String,
    #[doc = "Contexto adicional (campo, versão esperada e atual, org). Pode ser `null`."]
    pub details: Value,
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    details: Value,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            details: Value::Null,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_request", message)
    }

    pub fn invalid_id(campo: &'static str, value: &str, motivo: impl Into<String>) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "invalid_id",
            format!("`{value}` não é um id válido de {campo}"),
        )
        .with_details(json!({ "field": campo, "reason": motivo.into() }))
    }

    pub fn validation(campo: impl Into<String>, motivo: impl Into<String>) -> Self {
        let campo = campo.into();
        Self::new(
            StatusCode::BAD_REQUEST,
            "validation_error",
            format!("campo `{campo}` inválido"),
        )
        .with_details(json!({ "field": campo, "reason": motivo.into() }))
    }

    pub fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "informe um token válido em `Authorization: Bearer <token>`",
        )
    }

    pub fn forbidden(detail: &str) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", detail)
    }

    pub fn not_found(entity: &str, id: &str) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("{entity} `{id}` não encontrado"),
        )
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", message)
    }

    pub fn version_conflict(entity: &str, id: &str, expected: u64, current: u64) -> Self {
        Self::new(
            StatusCode::PRECONDITION_FAILED,
            "version_conflict",
            format!("{entity} `{id}` mudou desde a leitura"),
        )
        .with_details(json!({ "expected_version": expected, "current_version": current }))
    }

    pub fn if_match_required(descricao: &str) -> Self {
        Self::new(
            StatusCode::PRECONDITION_REQUIRED,
            "precondition_required",
            format!("envie o cabeçalho `If-Match` com a versão de {descricao}"),
        )
    }

    pub fn internal() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "erro interno do servidor",
        )
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorResponse {
            code: self.code,
            message: self.message,
            details: self.details,
        };
        let mut response = (self.status, Json(body)).into_response();
        if self.status == StatusCode::UNAUTHORIZED {
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                header::HeaderValue::from_static("Bearer"),
            );
        }
        response
    }
}

impl From<StoreError> for ApiError {
    fn from(error: StoreError) -> Self {
        match &error {
            StoreError::Domain(domain) => Self::from_domain(domain),
            other => {
                tracing::error!("erro" = %other, "falha no armazenamento ao atender a API");
                Self::internal()
            }
        }
    }
}

impl ApiError {
    fn from_domain(error: &DomainError) -> Self {
        match error {
            DomainError::InvalidId {
                entity,
                value,
                reason,
            } => Self::invalid_id(entity, value, reason.clone()),
            DomainError::NotFound { entity, id } => Self::not_found(entity, id),
            DomainError::VersionConflict {
                entity,
                id,
                expected,
                current,
            } => Self::version_conflict(entity, id, *expected, *current),
            DomainError::Validation { field, reason } => Self::validation(*field, reason.clone()),
            DomainError::Conflict { reason, .. } => Self::conflict(reason.clone()),
            DomainError::AgentInactive { id, status } => {
                Self::conflict(format!("agente `{id}` está {status}: não abre turno novo"))
            }
            other => Self::from_domain_code(other),
        }
    }

    fn from_domain_code(error: &DomainError) -> Self {
        let status = StatusCode::from_u16(error.http_status()).unwrap_or(StatusCode::BAD_REQUEST);
        Self::new(status, error.api_code(), error.to_string())
    }
}

pub fn id_from_url<T>(texto: &str) -> Result<T, ApiError>
where
    T: std::str::FromStr<Err = DomainError>,
    T: NamedId,
{
    texto
        .parse::<T>()
        .map_err(|error| ApiError::invalid_id(T::ENTITY, texto, error.to_string()))
}

pub trait NamedId {
    const ENTITY: &'static str;
}

macro_rules! named_id {
    ($ty:ty, $name:literal) => {
        impl NamedId for $ty {
            const ENTITY: &'static str = $name;
        }
    };
}

named_id!(poph_types::OrgId, "org");
named_id!(poph_types::AdminTokenId, "token");
named_id!(poph_types::ProductCategoryId, "categoria de produto");
named_id!(poph_types::DepartmentId, "departamento");
named_id!(poph_types::AgentId, "agente");
named_id!(poph_types::FlowId, "fluxo");
named_id!(poph_types::FileId, "arquivo");

named_id!(poph_types::ConversationId, "conversa");
named_id!(poph_types::RecordId, "ficha");
named_id!(poph_types::MessageId, "mensagem");
named_id!(poph_types::AttachmentId, "anexo");
named_id!(poph_types::ProductId, "produto");

named_id!(poph_types::IntegrationId, "integração");
