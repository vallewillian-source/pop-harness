use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordVerifier};
use axum::extract::FromRequestParts;
use axum::http::header;
use axum::http::request::Parts;
use poph_types::{AdminTokenId, OrgId};

use crate::error::ApiError;
use crate::state::ServerState;

pub const ORG_PREFIX: &str = "poph_";
pub const GLOBAL_PREFIX: &str = "pophg_";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub token_id: Option<AdminTokenId>,
    pub label: String,
    pub org: Option<OrgId>,
}

impl Identity {
    pub fn eh_global(&self) -> bool {
        self.org.is_none()
    }

    pub fn require_org(&self, org: OrgId) -> Result<(), ApiError> {
        match self.org {
            Some(own) if own != org => Err(ApiError::forbidden(
                "este token é de outra org: ele só enxerga a própria org",
            )),
            _ => Ok(()),
        }
    }

    pub fn require_global(&self) -> Result<(), ApiError> {
        if self.eh_global() {
            Ok(())
        } else {
            Err(ApiError::forbidden(
                "esta ação exige um token global (o token de org não cria nem remove orgs e tokens)",
            ))
        }
    }

    pub fn actor_id(&self) -> String {
        match self.token_id {
            Some(id) => id.to_string(),
            None => format!("global:{}", self.label),
        }
    }
}

impl FromRequestParts<ServerState> for Identity {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        estado: &ServerState,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(ApiError::unauthorized)?;
        let (scheme, value) = header.split_once(' ').ok_or_else(ApiError::unauthorized)?;
        if !scheme.eq_ignore_ascii_case("bearer") {
            return Err(ApiError::unauthorized());
        }
        let value = value.trim();
        if value.is_empty() {
            return Err(ApiError::unauthorized());
        }
        autenticar(value, estado).await
    }
}

pub async fn autenticar(value: &str, estado: &ServerState) -> Result<Identity, ApiError> {
    if value.starts_with(GLOBAL_PREFIX) {
        return autenticar_global(value, estado).await;
    }
    let rest = value
        .strip_prefix(ORG_PREFIX)
        .ok_or_else(ApiError::unauthorized)?;
    let (text_id, secret) = rest.split_once('_').ok_or_else(ApiError::unauthorized)?;
    let id: AdminTokenId = text_id.parse().map_err(|_| ApiError::unauthorized())?;
    let registro = estado
        .store
        .tokens()
        .by_id(id)
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::unauthorized)?;
    if registro.revoked_at.is_some() {
        return Err(ApiError::unauthorized());
    }
    let hash = registro.hash.clone();
    let secret = secret.to_owned();
    if !check_in_thread(secret, hash).await {
        return Err(ApiError::unauthorized());
    }
    Ok(Identity {
        token_id: Some(id),
        label: registro.label,
        org: registro.org_id,
    })
}

async fn autenticar_global(value: &str, estado: &ServerState) -> Result<Identity, ApiError> {
    if estado.global_tokens.is_empty() {
        return Err(ApiError::unauthorized());
    }
    let hashes: Vec<String> = estado
        .global_tokens
        .iter()
        .map(|token| token.hash.clone())
        .collect();
    let value = value.to_owned();
    let checked = tokio::task::spawn_blocking(move || {
        let mut found = None;
        for (position, hash) in hashes.iter().enumerate() {
            if check(&value, hash) {
                found = Some(position);
                break;
            }
        }
        found
    })
    .await
    .ok()
    .flatten();
    let Some(position) = checked else {
        return Err(ApiError::unauthorized());
    };
    let token = &estado.global_tokens[position];
    Ok(Identity {
        token_id: None,
        label: token.label.clone(),
        org: None,
    })
}

async fn check_in_thread(secret: String, hash: String) -> bool {
    tokio::task::spawn_blocking(move || check(&secret, &hash))
        .await
        .unwrap_or(false)
}

fn check(value: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(value.as_bytes(), &parsed)
        .is_ok()
}
