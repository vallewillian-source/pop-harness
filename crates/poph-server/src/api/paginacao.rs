//! Paginação por cursor (PRD §8.1): `?limit=50&cursor=<último id da página anterior>`.
//!
//! O cursor é o último id da página (UUID v7, ordenável por tempo) e é exclusivo. A resposta traz
//! `next_cursor` `null` quando acabou.

use poph_store::Pagina;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::erro::ApiError;

/// Parâmetros de listagem comuns a todas as coleções.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct ConsultaPagina {
    /// Quantos registros trazer (1 a 200). Padrão 50.
    pub limit: Option<u32>,
    /// Último id da página anterior.
    pub cursor: Option<String>,
}

impl ConsultaPagina {
    /// Converte para a página do armazenamento (cursor inválido vira 400).
    pub fn pagina(&self) -> Result<Pagina, ApiError> {
        let limite = self.limit.unwrap_or(50).clamp(1, 200);
        let apos = match &self.cursor {
            Some(texto) if !texto.is_empty() => {
                Some(texto.parse::<uuid::Uuid>().map_err(|_| {
                    ApiError::invalido("cursor inválido: use o `next_cursor` da resposta anterior")
                        .com_detalhes(serde_json::json!({ "field": "cursor", "value": texto }))
                })?)
            }
            _ => None,
        };
        Ok(Pagina { apos, limite })
    }
}

/// Página de resultados.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ListaJson<T> {
    /// Os registros desta página.
    pub items: Vec<T>,
    /// Cursor para a próxima página; `null` quando esta é a última.
    pub next_cursor: Option<String>,
}

impl<T> ListaJson<T> {
    /// Lista vazia (usada quando o token de org só enxerga a própria org).
    pub fn vazia() -> Self {
        Self {
            items: Vec::new(),
            next_cursor: None,
        }
    }
}

/// Próximo cursor: o id do último item quando a página veio cheia.
pub fn proximo_cursor<T, F>(items: &[T], limite: u32, id: F) -> Option<String>
where
    F: Fn(&T) -> String,
{
    if items.len() < limite as usize {
        return None;
    }
    items.last().map(id)
}
