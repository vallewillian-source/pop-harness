use poph_store::Page;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::erro::ApiError;

#[doc = " Parâmetros de listagem comuns a todas as coleções."]
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[schema(as = ConsultaPagina)]
pub struct PageQuery {
    #[doc = " Quantos registros trazer (1 a 200). Padrão 50."]
    pub limit: Option<u32>,
    #[doc = " Último id da página anterior."]
    pub cursor: Option<String>,
}

impl PageQuery {
    pub fn page(&self) -> Result<Page, ApiError> {
        let limit = self.limit.unwrap_or(50).clamp(1, 200);
        let after = match &self.cursor {
            Some(text) if !text.is_empty() => Some(text.parse::<uuid::Uuid>().map_err(|_| {
                ApiError::invalido("cursor inválido: use o `next_cursor` da resposta anterior")
                    .com_detalhes(serde_json::json!({ "field": "cursor", "value": text }))
            })?),
            _ => None,
        };
        Ok(Page { after, limit })
    }
}

#[doc = " Página de resultados."]
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = ListaJson)]
pub struct ListJson<T> {
    #[doc = " Os registros desta página."]
    pub items: Vec<T>,
    #[doc = " Cursor para a próxima página; `null` quando esta é a última."]
    pub next_cursor: Option<String>,
}

impl<T> ListJson<T> {
    pub fn is_empty() -> Self {
        Self {
            items: Vec::new(),
            next_cursor: None,
        }
    }
}

pub fn next_cursor<T, F>(items: &[T], limit: u32, id: F) -> Option<String>
where
    F: Fn(&T) -> String,
{
    if items.len() < limit as usize {
        return None;
    }
    items.last().map(id)
}
