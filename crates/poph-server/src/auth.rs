//! Autenticação da API administrativa (PRD §8.1, §14).
//!
//! Dois tipos de token, ambos com `Authorization: Bearer <valor>`:
//!
//! - **de org:** `poph_<uuid do token>_<segredo>`. O uuid localiza o registro em `admin_tokens`
//!   e o segredo é conferido contra o hash argon2 gravado. Revogado = não autentica.
//! - **global:** `pophg_<segredo>`, conferido contra o hash que está na configuração
//!   (`[admin].tokens_globais`). Não toca no banco e enxerga todas as orgs.
//!
//! O argon2 é caro de propósito (~dezenas de ms); a conferência roda em `spawn_blocking` para não
//! segurar o worker do tokio. O valor do token nunca é logado.

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordVerifier};
use axum::extract::FromRequestParts;
use axum::http::header;
use axum::http::request::Parts;
use poph_types::{AdminTokenId, OrgId};

use crate::erro::ApiError;
use crate::estado::Estado;

/// Prefixo do token de org (`poph_<uuid>_<segredo>`).
pub const PREFIXO_ORG: &str = "poph_";
/// Prefixo do token global (`pophg_<segredo>`).
pub const PREFIXO_GLOBAL: &str = "pophg_";

/// Quem está chamando a API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identidade {
    /// Id do token de org; `None` no token global.
    pub token_id: Option<AdminTokenId>,
    /// Rótulo do token (auditoria e log).
    pub rotulo: String,
    /// Org do token de org; `None` no token global.
    pub org: Option<OrgId>,
}

impl Identidade {
    /// `true` quando o token enxerga todas as orgs.
    pub fn eh_global(&self) -> bool {
        self.org.is_none()
    }

    /// Confere que o token pode mexer na org pedida.
    ///
    /// Token global passa em qualquer org; token de org passa só na própria, e errar dá 403 (não
    /// 404: o cliente sabe qual org pediu e precisa ver que foi barrado por escopo).
    pub fn exigir_org(&self, org: OrgId) -> Result<(), ApiError> {
        match self.org {
            Some(propria) if propria != org => Err(ApiError::proibido(
                "este token é de outra org: ele só enxerga a própria org",
            )),
            _ => Ok(()),
        }
    }

    /// Exige token global (criar org, excluir org, criar/revogar token).
    pub fn exigir_global(&self) -> Result<(), ApiError> {
        if self.eh_global() {
            Ok(())
        } else {
            Err(ApiError::proibido(
                "esta ação exige um token global (o token de org não cria nem remove orgs e tokens)",
            ))
        }
    }

    /// Valor para a auditoria (`admin_token` + id ou rótulo).
    pub fn ator_id(&self) -> String {
        match self.token_id {
            Some(id) => id.to_string(),
            None => format!("global:{}", self.rotulo),
        }
    }
}

impl FromRequestParts<Estado> for Identidade {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        estado: &Estado,
    ) -> Result<Self, Self::Rejection> {
        let cabecalho = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|valor| valor.to_str().ok())
            .ok_or_else(ApiError::nao_autorizado)?;
        let (esquema, valor) = cabecalho
            .split_once(' ')
            .ok_or_else(ApiError::nao_autorizado)?;
        if !esquema.eq_ignore_ascii_case("bearer") {
            return Err(ApiError::nao_autorizado());
        }
        let valor = valor.trim();
        if valor.is_empty() {
            return Err(ApiError::nao_autorizado());
        }
        autenticar(valor, estado).await
    }
}

/// Autentica o valor do `Bearer`. Público para os testes exercitarem o formato sem HTTP.
pub async fn autenticar(valor: &str, estado: &Estado) -> Result<Identidade, ApiError> {
    if valor.starts_with(PREFIXO_GLOBAL) {
        return autenticar_global(valor, estado).await;
    }
    let resto = valor
        .strip_prefix(PREFIXO_ORG)
        .ok_or_else(ApiError::nao_autorizado)?;
    let (texto_id, segredo) = resto.split_once('_').ok_or_else(ApiError::nao_autorizado)?;
    let id: AdminTokenId = texto_id.parse().map_err(|_| ApiError::nao_autorizado())?;
    let registro = estado
        .store
        .tokens()
        .por_id(id)
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::nao_autorizado)?;
    if registro.revogado_em.is_some() {
        return Err(ApiError::nao_autorizado());
    }
    let hash = registro.hash.clone();
    let segredo = segredo.to_owned();
    if !conferir_em_thread(segredo, hash).await {
        return Err(ApiError::nao_autorizado());
    }
    Ok(Identidade {
        token_id: Some(id),
        rotulo: registro.rotulo,
        org: registro.org_id,
    })
}

async fn autenticar_global(valor: &str, estado: &Estado) -> Result<Identidade, ApiError> {
    if estado.tokens_globais.is_empty() {
        return Err(ApiError::nao_autorizado());
    }
    let hashes: Vec<String> = estado
        .tokens_globais
        .iter()
        .map(|token| token.hash.clone())
        .collect();
    let valor = valor.to_owned();
    let conferido = tokio::task::spawn_blocking(move || {
        let mut achado = None;
        for (posicao, hash) in hashes.iter().enumerate() {
            if conferir(&valor, hash) {
                achado = Some(posicao);
                break;
            }
        }
        achado
    })
    .await
    .ok()
    .flatten();
    let Some(posicao) = conferido else {
        return Err(ApiError::nao_autorizado());
    };
    let token = &estado.tokens_globais[posicao];
    Ok(Identidade {
        token_id: None,
        rotulo: token.label.clone(),
        org: None,
    })
}

async fn conferir_em_thread(segredo: String, hash: String) -> bool {
    tokio::task::spawn_blocking(move || conferir(&segredo, &hash))
        .await
        .unwrap_or(false)
}

fn conferir(valor: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(valor.as_bytes(), &parsed)
        .is_ok()
}
