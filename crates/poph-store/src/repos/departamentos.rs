//! Repositório de departamentos (destino de conversa e de transferência).

use poph_types::{DepartmentId, OrgId};
use rusqlite::{OptionalExtension, Row, params};

use crate::Store;
use crate::erro::{ErroStore, Resultado};
use crate::modelo::{Departamento, MudancaDepartamento, NovoDepartamento, Pagina, Status};
use crate::repos::{conflito_ou_ausente, id_da_coluna, status_da_coluna, u64_da_coluna};

const COLUNAS: &str = "id, org_id, name, description, status, version, created_at, updated_at";

/// Acesso aos departamentos.
pub struct Departamentos<'a>(pub(crate) &'a Store);

impl Departamentos<'_> {
    /// Cria o departamento na org.
    pub fn criar(&self, org: OrgId, novo: NovoDepartamento) -> Resultado<Departamento> {
        let id = DepartmentId::generate();
        let agora = self.0.agora();
        let texto_id = id.to_string();
        let texto_org = org.to_string();
        self.0.escrita(move |conn| {
            conn.execute(
                "INSERT INTO departments
                    (id, org_id, name, description, status, version, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 'active', 1, ?5, ?5)",
                params![texto_id, texto_org, novo.nome, novo.descricao, agora],
            )
            .map_err(|erro| ErroStore::da_escrita("nome", erro))?;
            Ok(())
        })?;
        self.obter(org, id)
    }

    /// Busca pelo id dentro da org.
    pub fn obter(&self, org: OrgId, id: DepartmentId) -> Resultado<Departamento> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let encontrado = self.0.leitura(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUNAS} FROM departments WHERE id = ?1 AND org_id = ?2"),
                params![texto_id, texto_org],
                linha,
            )
            .optional()
            .map_err(ErroStore::from)
        })?;
        super::exigir(encontrado, "departamento", &id.to_string())
    }

    /// Busca pelo **nome** dentro da org, sem diferenciar maiúsculas (o índice único é
    /// `(org_id, lower(name))`). É o que o efeito `encaminhar(departamento)` precisa: a IR guarda o
    /// nome que a organização escreveu, não o id.
    ///
    /// Nome que não existe devolve `None` — quem chama decide se é falha (o efeito é) ou se é caso
    /// de usar outro departamento.
    pub fn por_nome(&self, org: OrgId, nome: &str) -> Resultado<Option<Departamento>> {
        let texto_org = org.to_string();
        let nome = nome.trim().to_lowercase();
        self.0.leitura(move |conn| {
            conn.query_row(
                &format!(
                    "SELECT {COLUNAS} FROM departments
                      WHERE org_id = ?1 AND lower(name) = ?2"
                ),
                params![texto_org, nome],
                linha,
            )
            .optional()
            .map_err(ErroStore::from)
        })
    }

    /// Lista os departamentos da org (todos; o filtro de ativo é do chamador).
    pub fn listar(&self, org: OrgId, pagina: Pagina) -> Resultado<Vec<Departamento>> {
        let texto_org = org.to_string();
        let apos = pagina.apos.map(|id| id.to_string());
        let limite = i64::from(pagina.limite);
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(&format!(
                "SELECT {COLUNAS} FROM departments
                  WHERE org_id = ?1 AND (?2 IS NULL OR id > ?2)
                  ORDER BY id LIMIT ?3"
            ))?;
            let linhas = consulta.query_map(params![texto_org, apos, limite], linha)?;
            let mut departamentos = Vec::new();
            for linha in linhas {
                departamentos.push(linha?);
            }
            Ok(departamentos)
        })
    }

    /// Altera com `If-Match`.
    pub fn atualizar(
        &self,
        org: OrgId,
        id: DepartmentId,
        esperada: u64,
        mudanca: MudancaDepartamento,
    ) -> Resultado<Departamento> {
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let status = mudanca.status.map(Status::como_texto);
        let esperada_i64 = esperada as i64;
        let efeito = self.0.escrita(move |conn| {
            conn.execute(
                "UPDATE departments
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

    /// Exclui o departamento (agentes que o usavam ficam com o campo nulo).
    pub fn excluir(&self, org: OrgId, id: DepartmentId) -> Resultado<()> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let apagou = self.0.escrita(move |conn| {
            conn.execute(
                "DELETE FROM departments WHERE org_id = ?1 AND id = ?2",
                params![texto_org, texto_id],
            )
            .map_err(ErroStore::from)
        })?;
        if apagou == 0 {
            return Err(ErroStore::Dominio(poph_types::DomainError::NaoEncontrado {
                entidade: "departamento",
                id: id.to_string(),
            }));
        }
        Ok(())
    }

    fn conflito(&self, org: OrgId, id: DepartmentId, esperada: u64) -> ErroStore {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        self.0
            .leitura(move |conn| {
                Ok(conflito_ou_ausente(
                    conn,
                    "departments",
                    "departamento",
                    &texto_id,
                    &texto_org,
                    esperada,
                ))
            })
            .unwrap_or_else(|erro| erro)
    }
}

fn linha(linha: &Row<'_>) -> rusqlite::Result<Departamento> {
    Ok(Departamento {
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
