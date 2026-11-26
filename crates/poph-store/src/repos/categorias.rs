//! Repositório de categorias de produto (o que se vincula ao agente).

use poph_types::{OrgId, ProductCategoryId};
use rusqlite::{OptionalExtension, Row, params};

use crate::Store;
use crate::erro::{ErroStore, Resultado};
use crate::modelo::{CategoriaProduto, MudancaCategoria, NovaCategoria, Pagina, Status};
use crate::repos::{conflito_ou_ausente, id_da_coluna, status_da_coluna, u64_da_coluna};

const COLUNAS: &str = "id, org_id, name, description, status, version, created_at, updated_at";

/// Acesso às categorias de produto.
pub struct Categorias<'a>(pub(crate) &'a Store);

impl Categorias<'_> {
    /// Cria a categoria na org.
    pub fn criar(&self, org: OrgId, nova: NovaCategoria) -> Resultado<CategoriaProduto> {
        let id = ProductCategoryId::generate();
        let agora = self.0.agora();
        let texto_id = id.to_string();
        let texto_org = org.to_string();
        self.0.escrita(move |conn| {
            conn.execute(
                "INSERT INTO product_categories
                    (id, org_id, name, description, status, version, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 'active', 1, ?5, ?5)",
                params![texto_id, texto_org, nova.nome, nova.descricao, agora],
            )
            .map_err(|erro| ErroStore::da_escrita("nome", erro))?;
            Ok(())
        })?;
        self.obter(org, id)
    }

    /// Busca pelo id dentro da org.
    pub fn obter(&self, org: OrgId, id: ProductCategoryId) -> Resultado<CategoriaProduto> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let encontrada = self.0.leitura(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUNAS} FROM product_categories WHERE id = ?1 AND org_id = ?2"),
                params![texto_id, texto_org],
                linha,
            )
            .optional()
            .map_err(ErroStore::from)
        })?;
        super::exigir(encontrada, "categoria de produto", &id.to_string())
    }

    /// Lista as categorias da org.
    pub fn listar(&self, org: OrgId, pagina: Pagina) -> Resultado<Vec<CategoriaProduto>> {
        let texto_org = org.to_string();
        let apos = pagina.apos.map(|id| id.to_string());
        let limite = i64::from(pagina.limite);
        self.0.leitura(move |conn| {
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
        mudanca: MudancaCategoria,
    ) -> Resultado<CategoriaProduto> {
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let status = mudanca.status.map(Status::como_texto);
        let esperada_i64 = esperada as i64;
        let efeito = self.0.escrita(move |conn| {
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
                    mudanca.nome,
                    mudanca.descricao,
                    status,
                    agora,
                    esperada_i64
                ],
            )
            .map_err(|erro| ErroStore::da_escrita("nome", erro))
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
        let apagou = self.0.escrita(move |conn| {
            conn.execute(
                "DELETE FROM product_categories WHERE org_id = ?1 AND id = ?2",
                params![texto_org, texto_id],
            )
            .map_err(ErroStore::from)
        })?;
        if apagou == 0 {
            return Err(nao_encontrada(org, id));
        }
        Ok(())
    }

    fn conflito(&self, org: OrgId, id: ProductCategoryId, esperada: u64) -> ErroStore {
        self.0
            .leitura(move |conn| {
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

fn nao_encontrada(org: OrgId, id: ProductCategoryId) -> ErroStore {
    let _ = org;
    ErroStore::Dominio(poph_types::DomainError::NotFound {
        entity: "categoria de produto",
        id: id.to_string(),
    })
}

fn linha(linha: &Row<'_>) -> rusqlite::Result<CategoriaProduto> {
    Ok(CategoriaProduto {
        id: id_da_coluna(linha, 0)?,
        org_id: id_da_coluna(linha, 1)?,
        nome: linha.get(2)?,
        descricao: linha.get(3)?,
        status: status_da_coluna(linha, 4)?,
        versao: u64_da_coluna(linha, 5)?,
        criado_em: linha.get(6)?,
        atualizado_em: linha.get(7)?,
    })
}
