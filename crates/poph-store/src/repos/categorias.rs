//! Repositório de categorias de produto (o que se vincula ao agente).

use poph_types::{OrgId, ProductCategoryId};
use rusqlite::{OptionalExtension, Row, params};

use crate::Store;
use crate::error::{Resultado, StoreError};
use crate::model::{CategoryChange, NewCategory, Page, ProductCategory, Status};
use crate::repos::{conflito_ou_ausente, id_da_coluna, status_da_coluna, u64_da_coluna};

const COLUNAS: &str = "id, org_id, name, description, status, version, created_at, updated_at";

/// Acesso às categorias de produto.
pub struct Categorias<'a>(pub(crate) &'a Store);

impl Categorias<'_> {
    /// Cria a categoria na org.
    pub fn criar(&self, org: OrgId, nova: NewCategory) -> Resultado<ProductCategory> {
        let id = ProductCategoryId::generate();
        let agora = self.0.now();
        let texto_id = id.to_string();
        let texto_org = org.to_string();
        self.0.write(move |conn| {
            conn.execute(
                "INSERT INTO product_categories
                    (id, org_id, name, description, status, version, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 'active', 1, ?5, ?5)",
                params![texto_id, texto_org, nova.name, nova.description, agora],
            )
            .map_err(|erro| StoreError::from_write("nome", erro))?;
            Ok(())
        })?;
        self.obter(org, id)
    }

    /// Busca pelo id dentro da org.
    pub fn obter(&self, org: OrgId, id: ProductCategoryId) -> Resultado<ProductCategory> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let encontrada = self.0.read(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUNAS} FROM product_categories WHERE id = ?1 AND org_id = ?2"),
                params![texto_id, texto_org],
                linha,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(encontrada, "categoria de produto", &id.to_string())
    }

    /// Lista as categorias da org.
    pub fn listar(&self, org: OrgId, pagina: Page) -> Resultado<Vec<ProductCategory>> {
        let texto_org = org.to_string();
        let apos = pagina.after.map(|id| id.to_string());
        let limite = i64::from(pagina.limit);
        self.0.read(move |conn| {
            let mut consulta = conn.prepare(&format!(
                "SELECT {COLUNAS} FROM product_categories
                  WHERE org_id = ?1 AND (?2 IS NULL OR id > ?2)
                  ORDER BY id LIMIT ?3"
            ))?;
            let linhas = consulta.query_map(params![texto_org, apos, limite], linha)?;
            let mut categorias = Vec::new();
            for linha in linhas {
                categorias.push(linha?);
            }
            Ok(categorias)
        })
    }

    /// Altera com `If-Match`.
    pub fn atualizar(
        &self,
        org: OrgId,
        id: ProductCategoryId,
        esperada: u64,
        mudanca: CategoryChange,
    ) -> Resultado<ProductCategory> {
        let agora = self.0.now();
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let status = mudanca.status.map(Status::as_text);
        let esperada_i64 = esperada as i64;
        let efeito = self.0.write(move |conn| {
            conn.execute(
                "UPDATE product_categories
                    SET name = COALESCE(?3, name),
                        description = COALESCE(?4, description),
                        status = COALESCE(?5, status),
                        version = version + 1,
                        updated_at = ?6
                  WHERE org_id = ?1 AND id = ?2 AND version = ?7",
                params![
                    texto_org,
                    texto_id,
                    mudanca.name,
                    mudanca.description,
                    status,
                    agora,
                    esperada_i64
                ],
            )
            .map_err(|erro| StoreError::from_write("nome", erro))
        })?;
        if efeito == 0 {
            return Err(self.conflito(org, id, esperada));
        }
        self.obter(org, id)
    }

    /// Exclui a categoria (os produtos dela saem junto; vínculos com agentes também).
    pub fn excluir(&self, org: OrgId, id: ProductCategoryId) -> Resultado<()> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let apagou = self.0.write(move |conn| {
            conn.execute(
                "DELETE FROM product_categories WHERE org_id = ?1 AND id = ?2",
                params![texto_org, texto_id],
            )
            .map_err(StoreError::from)
        })?;
        if apagou == 0 {
            return Err(nao_encontrada(org, id));
        }
        Ok(())
    }

    fn conflito(&self, org: OrgId, id: ProductCategoryId, esperada: u64) -> StoreError {
        self.0
            .read(move |conn| {
                Ok(conflito_ou_ausente(
                    conn,
                    "product_categories",
                    "categoria de produto",
                    &id.to_string(),
                    &org.to_string(),
                    esperada,
                ))
            })
            .unwrap_or_else(|erro| erro)
    }
}

fn nao_encontrada(org: OrgId, id: ProductCategoryId) -> StoreError {
    let _ = org;
    StoreError::Domain(poph_types::DomainError::NotFound {
        entity: "categoria de produto",
        id: id.to_string(),
    })
}

fn linha(linha: &Row<'_>) -> rusqlite::Result<ProductCategory> {
    Ok(ProductCategory {
        id: id_da_coluna(linha, 0)?,
        org_id: id_da_coluna(linha, 1)?,
        name: linha.get(2)?,
        description: linha.get(3)?,
        status: status_da_coluna(linha, 4)?,
        version: u64_da_coluna(linha, 5)?,
        created_at: linha.get(6)?,
        updated_at: linha.get(7)?,
    })
}
