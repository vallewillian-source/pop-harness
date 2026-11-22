//! Repositório das instruções do agente (JSON estruturado + markdown composto).
//!
//! O par é o que o motor lê: o markdown é o prefixo estável do prompt (PRD §6.1) e o JSON é a
//! fonte do compositor (HR-09). `derivado_do_json` guarda a distinção que o POP Atende faz: texto
//! gerado pelo formulário pode ser recomposto; texto editado à mão nunca é sobrescrito sem ordem.

use poph_types::{AgentId, OrgId};
use rusqlite::{OptionalExtension, Row, params};

use crate::Store;
use crate::erro::{ErroStore, Resultado};
use crate::modelo::{Instrucoes, Pagina};
use crate::repos::{conflito_ou_ausente, id_da_coluna, u64_da_coluna};

const COLUNAS: &str = "agent_id, org_id, structured_json, markdown, derived_from_json, version, created_at, updated_at";

/// Acesso às instruções.
pub struct InstrucoesRepo<'a>(pub(crate) &'a Store);

impl InstrucoesRepo<'_> {
    /// Lê as instruções do agente.
    pub fn obter(&self, org: OrgId, agent: AgentId) -> Resultado<Instrucoes> {
        let texto_org = org.to_string();
        let texto_agente = agent.to_string();
        let encontradas = self.0.leitura(move |conn| {
            conn.query_row(
                &format!(
                    "SELECT {COLUNAS} FROM agent_instructions WHERE org_id = ?1 AND agent_id = ?2"
                ),
                params![texto_org, texto_agente],
                linha,
            )
            .optional()
            .map_err(ErroStore::from)
        })?;
        super::exigir(encontradas, "instruções do agente", &agent.to_string())
    }

    /// Versão das instruções do agente, ou `None` quando ele ainda não tem instruções.
    ///
    /// É a segunda metade da chave de invalidação da config quente (HR-11): editar só o texto do
    /// prompt não muda a versão do agente, mas muda o que o LLM recebe.
    pub fn versao(&self, org: OrgId, agent: AgentId) -> Resultado<Option<u64>> {
        let texto_org = org.to_string();
        let texto_agente = agent.to_string();
        self.0.leitura(move |conn| {
            conn.query_row(
                "SELECT version FROM agent_instructions WHERE org_id = ?1 AND agent_id = ?2",
                params![texto_org, texto_agente],
                |linha| u64_da_coluna(linha, 0),
            )
            .optional()
            .map_err(ErroStore::from)
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
    ) -> Resultado<Instrucoes> {
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let texto_agente = agent.to_string();
        let estruturado_json = estruturado_json.to_owned();
        let markdown = markdown.to_owned();
        let derivado = i64::from(derivado_do_json);
        self.0.escrita(move |conn| {
            let atual: Option<i64> = conn
                .query_row(
                    "SELECT version FROM agent_instructions WHERE org_id = ?1 AND agent_id = ?2",
                    params![texto_org, texto_agente],
                    |linha| linha.get(0),
                )
                .optional()
                .map_err(ErroStore::from)?;
            match atual {
                Some(atual) => {
                    if atual.max(0) as u64 != esperada {
                        return Err(ErroStore::Dominio(
                            poph_types::DomainError::ConflitoDeVersao {
                                entidade: "instruções do agente",
                                id: texto_agente,
                                esperada,
                                atual: atual.max(0) as u64,
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
                    .map_err(ErroStore::from)?;
                }
                None => {
                    if esperada != 0 {
                        return Err(ErroStore::Dominio(
                            poph_types::DomainError::ConflitoDeVersao {
                                entidade: "instruções do agente",
                                id: texto_agente,
                                esperada,
                                atual: 0,
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
                        .map_err(ErroStore::from)?;
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
                    .map_err(ErroStore::from)?;
                }
            }
            Ok(())
        })?;
        self.obter(org, agent)
    }

    /// Página de instruções da org, em ordem de agente.
    pub fn listar(&self, org: OrgId, pagina: Pagina) -> Resultado<Vec<Instrucoes>> {
        let texto_org = org.to_string();
        let apos = pagina.apos.map(|id| id.to_string());
        let limite = i64::from(pagina.limite);
        self.0.leitura(move |conn| {
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
        let apagou = self.0.escrita(move |conn| {
            conn.execute(
                "DELETE FROM agent_instructions WHERE org_id = ?1 AND agent_id = ?2",
                params![texto_org, texto_agente],
            )
            .map_err(ErroStore::from)
        })?;
        if apagou == 0 {
            return Err(ErroStore::Dominio(poph_types::DomainError::NaoEncontrado {
                entidade: "instruções do agente",
                id: agent.to_string(),
            }));
        }
        Ok(())
    }
}

fn linha(linha: &Row<'_>) -> rusqlite::Result<Instrucoes> {
    Ok(Instrucoes {
        agent_id: id_da_coluna(linha, 0)?,
        org_id: id_da_coluna(linha, 1)?,
        estruturado_json: linha.get(2)?,
        markdown: linha.get(3)?,
        derivado_do_json: linha.get::<_, i64>(4)? != 0,
        versao: u64_da_coluna(linha, 5)?,
        criado_em: linha.get(6)?,
        atualizado_em: linha.get(7)?,
    })
}
