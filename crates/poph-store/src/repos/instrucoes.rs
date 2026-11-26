//! Repositório das instruções do agente (JSON estruturado + markdown composto).
//!
//! O par é o que o motor lê: o markdown é o prefixo estável do prompt (PRD §6.1) e o JSON é a
//! fonte do compositor (HR-09). `derivado_do_json` guarda a distinção que o POP Atende faz: texto
//! gerado pelo formulário pode ser recomposto; texto editado à mão nunca é sobrescrito sem ordem.

use poph_types::{AgentId, OrgId};
use rusqlite::{OptionalExtension, Row, params};

use crate::Store;
use crate::error::{Resultado, StoreError};
use crate::model::{Instructions, Page};
use crate::repos::{conflito_ou_ausente, id_da_coluna, u64_da_coluna};

const COLUNAS: &str = "agent_id, org_id, structured_json, markdown, derived_from_json, version, created_at, updated_at";

/// Acesso às instruções.
pub struct InstrucoesRepo<'a>(pub(crate) &'a Store);

impl InstrucoesRepo<'_> {
    /// Lê as instruções do agente.
    pub fn obter(&self, org: OrgId, agent: AgentId) -> Resultado<Instructions> {
        let texto_org = org.to_string();
        let texto_agente = agent.to_string();
        let encontradas = self.0.read(move |conn| {
            conn.query_row(
                &format!(
                    "SELECT {COLUNAS} FROM agent_instructions WHERE org_id = ?1 AND agent_id = ?2"
                ),
                params![texto_org, texto_agente],
                linha,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(encontradas, "instruções do agente", &agent.to_string())
    }

    /// Versão das instruções do agente, ou `None` quando ele ainda não tem instruções.
    ///
    /// É a segunda metade da chave de invalidação da config quente (HR-11): editar só o texto do
    /// prompt não muda a versão do agente, mas muda o que o LLM recebe.
    pub fn versao(&self, org: OrgId, agent: AgentId) -> Resultado<Option<u64>> {
        let texto_org = org.to_string();
        let texto_agente = agent.to_string();
        self.0.read(move |conn| {
            conn.query_row(
                "SELECT version FROM agent_instructions WHERE org_id = ?1 AND agent_id = ?2",
                params![texto_org, texto_agente],
                |linha| u64_da_coluna(linha, 0),
            )
            .optional()
            .map_err(StoreError::from)
        })
    }

    /// Grava as instruções. `esperada = 0` cria (e falha se já existir); qualquer outro valor
    /// exige que a versão atual seja essa.
    pub fn salvar(
        &self,
        org: OrgId,
        agent: AgentId,
        esperada: u64,
        estruturado_json: &str,
        markdown: &str,
        derivado_do_json: bool,
    ) -> Resultado<Instructions> {
        let agora = self.0.now();
        let texto_org = org.to_string();
        let texto_agente = agent.to_string();
        let estruturado_json = estruturado_json.to_owned();
        let markdown = markdown.to_owned();
        let derivado = i64::from(derivado_do_json);
        self.0.write(move |conn| {
            let atual: Option<i64> = conn
                .query_row(
                    "SELECT version FROM agent_instructions WHERE org_id = ?1 AND agent_id = ?2",
                    params![texto_org, texto_agente],
                    |linha| linha.get(0),
                )
                .optional()
                .map_err(StoreError::from)?;
            match atual {
                Some(atual) => {
                    if atual.max(0) as u64 != esperada {
                        return Err(StoreError::Domain(
                            poph_types::DomainError::VersionConflict {
                                entity: "instruções do agente",
                                id: texto_agente,
                                expected: esperada,
                                current: atual.max(0) as u64,
                            },
                        ));
                    }
                    conn.execute(
                        "UPDATE agent_instructions
                            SET structured_json = ?3, markdown = ?4, derived_from_json = ?5,
                                version = version + 1, updated_at = ?6
                          WHERE org_id = ?1 AND agent_id = ?2",
                        params![
                            texto_org,
                            texto_agente,
                            estruturado_json,
                            markdown,
                            derivado,
                            agora
                        ],
                    )
                    .map_err(StoreError::from)?;
                }
                None => {
                    if esperada != 0 {
                        return Err(StoreError::Domain(
                            poph_types::DomainError::VersionConflict {
                                entity: "instruções do agente",
                                id: texto_agente,
                                expected: esperada,
                                current: 0,
                            },
                        ));
                    }
                    let existe_agente: Option<i64> = conn
                        .query_row(
                            "SELECT 1 FROM agents WHERE org_id = ?1 AND id = ?2",
                            params![texto_org, texto_agente],
                            |linha| linha.get(0),
                        )
                        .optional()
                        .map_err(StoreError::from)?;
                    if existe_agente.is_none() {
                        return Err(conflito_ou_ausente(
                            conn,
                            "agents",
                            "agente",
                            &texto_agente,
                            &texto_org,
                            0,
                        ));
                    }
                    conn.execute(
                        "INSERT INTO agent_instructions
                            (agent_id, org_id, structured_json, markdown, derived_from_json,
                             version, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?6)",
                        params![
                            texto_agente,
                            texto_org,
                            estruturado_json,
                            markdown,
                            derivado,
                            agora
                        ],
                    )
                    .map_err(StoreError::from)?;
                }
            }
            Ok(())
        })?;
        self.obter(org, agent)
    }

    /// Página de instruções da org, em ordem de agente.
    pub fn listar(&self, org: OrgId, pagina: Page) -> Resultado<Vec<Instructions>> {
        let texto_org = org.to_string();
        let apos = pagina.after.map(|id| id.to_string());
        let limite = i64::from(pagina.limit);
        self.0.read(move |conn| {
            let mut consulta = conn.prepare(&format!(
                "SELECT {COLUNAS} FROM agent_instructions
                  WHERE org_id = ?1 AND (?2 IS NULL OR agent_id > ?2)
                  ORDER BY agent_id LIMIT ?3"
            ))?;
            let linhas = consulta.query_map(params![texto_org, apos, limite], linha)?;
            let mut instrucoes = Vec::new();
            for item in linhas {
                instrucoes.push(item?);
            }
            Ok(instrucoes)
        })
    }

    /// Apaga as instruções do agente (o agente continua existindo, sem prompt composto).
    pub fn apagar(&self, org: OrgId, agent: AgentId) -> Resultado<()> {
        let texto_org = org.to_string();
        let texto_agente = agent.to_string();
        let apagou = self.0.write(move |conn| {
            conn.execute(
                "DELETE FROM agent_instructions WHERE org_id = ?1 AND agent_id = ?2",
                params![texto_org, texto_agente],
            )
            .map_err(StoreError::from)
        })?;
        if apagou == 0 {
            return Err(StoreError::Domain(poph_types::DomainError::NotFound {
                entity: "instruções do agente",
                id: agent.to_string(),
            }));
        }
        Ok(())
    }
}

fn linha(linha: &Row<'_>) -> rusqlite::Result<Instructions> {
    Ok(Instructions {
        agent_id: id_da_coluna(linha, 0)?,
        org_id: id_da_coluna(linha, 1)?,
        structured_json: linha.get(2)?,
        markdown: linha.get(3)?,
        derived_from_json: linha.get::<_, i64>(4)? != 0,
        version: u64_da_coluna(linha, 5)?,
        created_at: linha.get(6)?,
        updated_at: linha.get(7)?,
    })
}
