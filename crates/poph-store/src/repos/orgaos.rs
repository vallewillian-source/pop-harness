//! Repositório de orgs. A org é a raiz do isolamento: as demais tabelas apontam para ela.

use poph_types::OrgId;
use rusqlite::{OptionalExtension, Row, params};

use crate::Store;
use crate::error::{Resultado, StoreError};
use crate::model::{NewOrg, Org, OrgChange, Page, Status};
use crate::repos::{id_da_coluna, status_da_coluna, u64_da_coluna};

const COLUNAS: &str = "id, name, slug, status, config_json, version, created_at, updated_at";

/// Acesso às orgs.
pub struct Orgaos<'a>(pub(crate) &'a Store);

impl Orgaos<'_> {
    /// Cria a org (status inicial `active`).
    pub fn criar(&self, nova: NewOrg) -> Resultado<Org> {
        let id = OrgId::generate();
        let agora = self.0.now();
        let config_json = if nova.config_json.trim().is_empty() {
            "{}".to_owned()
        } else {
            nova.config_json
        };
        let texto_id = id.to_string();
        self.0.write(move |conn| {
            conn.execute(
                "INSERT INTO orgs (id, name, slug, status, config_json, version, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 'active', ?4, 1, ?5, ?5)",
                params![texto_id, nova.name, nova.slug, config_json, agora],
            )
            .map_err(|erro| StoreError::from_write("slug", erro))?;
            Ok(())
        })?;
        self.get(id)
    }

    /// Busca pelo id.
    pub fn get(&self, id: OrgId) -> Resultado<Org> {
        let texto = id.to_string();
        let encontrada = self.0.read(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUNAS} FROM orgs WHERE id = ?1"),
                params![texto],
                linha,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(encontrada, "org", &id.to_string())
    }

    /// Busca pelo slug (identificador estável na URL).
    pub fn por_slug(&self, slug: &str) -> Resultado<Org> {
        let procurado = slug.to_owned();
        let filtro = procurado.clone();
        let encontrada = self.0.read(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUNAS} FROM orgs WHERE slug = ?1"),
                params![procurado],
                linha,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(encontrada, "org", &slug_para_id(filtro))
    }

    /// Lista em ordem de criação (cursor pelo id).
    pub fn listar(&self, pagina: Page) -> Resultado<Vec<Org>> {
        let apos = pagina.after.map(|id| id.to_string());
        let limite = i64::from(pagina.limit);
        self.0.read(move |conn| {
            let mut consulta = conn.prepare(&format!(
                "SELECT {COLUNAS} FROM orgs
                 WHERE (?1 IS NULL OR id > ?1)
                 ORDER BY id LIMIT ?2"
            ))?;
            let linhas = consulta.query_map(params![apos, limite], linha)?;
            let mut orgs = Vec::new();
            for linha in linhas {
                orgs.push(linha?);
            }
            Ok(orgs)
        })
    }

    /// Altera com `If-Match`: `esperada` é a versão lida antes.
    pub fn atualizar(&self, id: OrgId, esperada: u64, mudanca: OrgChange) -> Resultado<Org> {
        let agora = self.0.now();
        let texto_id = id.to_string();
        let status = mudanca.status.map(Status::as_text);
        let esperada_i64 = esperada as i64;
        let efeito = self.0.write(move |conn| {
            conn.execute(
                "UPDATE orgs
                    SET name = COALESCE(?3, name),
                        status = COALESCE(?4, status),
                        config_json = COALESCE(?5, config_json),
                        version = version + 1,
                        updated_at = ?6
                  WHERE id = ?1 AND version = ?2",
                params![
                    texto_id,
                    esperada_i64,
                    mudanca.name,
                    status,
                    mudanca.config_json,
                    agora
                ],
            )
            .map_err(|erro| StoreError::from_write("slug", erro))
        })?;
        if efeito == 0 {
            return Err(conflito_de_org(self.0, id, esperada));
        }
        self.get(id)
    }

    /// Exclui a org **sem** conteúdo. Recusa com erro de conflito quando há agentes nela.
    ///
    /// A conferência e o `DELETE` rodam na mesma transação: não existe janela em que outro
    /// escritor coloque um agente entre a contagem e a exclusão. Quem quer apagar de qualquer
    /// forma (CLI com `--force`) usa [`Orgaos::excluir_forcado`].
    pub fn excluir(&self, id: OrgId) -> Resultado<()> {
        let texto = id.to_string();
        let conferido = texto.clone();
        let apagou = self.0.write(move |conn| {
            let agentes: i64 = conn
                .query_row(
                    "SELECT count(*) FROM agents WHERE org_id = ?1",
                    params![conferido],
                    |linha| linha.get(0),
                )
                .map_err(StoreError::from)?;
            if agentes > 0 {
                return Err(StoreError::Domain(poph_types::DomainError::Conflict {
                    entity: "org",
                    id: conferido.clone(),
                    reason: format!(
                        "{agentes} agente(s) na org; exclua os agentes antes, ou confirme com --force"
                    ),
                }));
            }
            conn.execute("DELETE FROM orgs WHERE id = ?1", params![conferido])
                .map_err(StoreError::from)
        })?;
        if apagou == 0 {
            return Err(StoreError::Domain(poph_types::DomainError::NotFound {
                entity: "org",
                id: id.to_string(),
            }));
        }
        Ok(())
    }

    /// Exclui a org e tudo que está dentro dela, mesmo com agentes. Devolve quantos agentes existiam.
    ///
    /// É o caminho da CLI com `--force`; a contagem vem da mesma transação do `DELETE`.
    pub fn excluir_forcado(&self, id: OrgId) -> Resultado<u64> {
        let texto = id.to_string();
        let conferido = texto.clone();
        let (apagou, agentes) = self.0.write(move |conn| {
            let agentes: i64 = conn
                .query_row(
                    "SELECT count(*) FROM agents WHERE org_id = ?1",
                    params![conferido],
                    |linha| linha.get(0),
                )
                .map_err(StoreError::from)?;
            let apagou = conn
                .execute("DELETE FROM orgs WHERE id = ?1", params![conferido])
                .map_err(StoreError::from)?;
            Ok((apagou, agentes))
        })?;
        if apagou == 0 {
            return Err(StoreError::Domain(poph_types::DomainError::NotFound {
                entity: "org",
                id: texto,
            }));
        }
        Ok(agentes.max(0) as u64)
    }
}

fn conflito_de_org(store: &Store, id: OrgId, esperada: u64) -> StoreError {
    let texto = id.to_string();
    let atual: Result<Option<i64>, _> = store.read({
        let texto = texto.clone();
        move |conn| {
            conn.query_row(
                "SELECT version FROM orgs WHERE id = ?1",
                params![texto],
                |linha| linha.get(0),
            )
            .optional()
            .map_err(StoreError::from)
        }
    });
    match atual {
        Ok(Some(atual)) => StoreError::Domain(poph_types::DomainError::VersionConflict {
            entity: "org",
            id: texto,
            expected: esperada,
            current: atual.max(0) as u64,
        }),
        _ => StoreError::Domain(poph_types::DomainError::NotFound {
            entity: "org",
            id: texto,
        }),
    }
}

fn slug_para_id(slug: String) -> String {
    format!("slug `{slug}`")
}

fn linha(linha: &Row<'_>) -> rusqlite::Result<Org> {
    Ok(Org {
        id: id_da_coluna(linha, 0)?,
        name: linha.get(1)?,
        slug: linha.get(2)?,
        status: status_da_coluna(linha, 3)?,
        config_json: linha.get(4)?,
        version: u64_da_coluna(linha, 5)?,
        created_at: linha.get(6)?,
        updated_at: linha.get(7)?,
    })
}
