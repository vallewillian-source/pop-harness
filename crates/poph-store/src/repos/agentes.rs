//! Repositório de agentes, incluindo o vínculo com categorias de produto.
//!
//! Os campos são os que a V1 mantém da PARIDADE_AGENTE.md (marcados `V1`): identidade, status,
//! tipo, departamento padrão e de transferência humano, provedor/modelo e parâmetros do LLM,
//! janela de agrupamento, limites do laço de ferramentas, top-k/corte do RAG e flags do Jev.
//! Regras de rota por intenção não existem de propósito (decisão D-06).

use std::collections::HashMap;
use std::path::Path;

use poph_types::{AgentId, OrgId, ProductCategoryId};
use rusqlite::{OptionalExtension, Row, named_params, params};

use crate::Store;
use crate::error::{Resultado, StoreError};
use crate::model::{AgentChange, Agente, NewAgent, Page, Status};
use crate::repos::{
    conflito_ou_ausente, id_da_coluna, id_opcional, pertence, status_da_coluna, u32_da_coluna,
    u64_da_coluna,
};

pub fn agentes_por_provedor_em(caminho: &Path) -> Resultado<Vec<(String, u64)>> {
    let conn = crate::connection::open_for_read(caminho)?;
    let mut consulta =
        conn.prepare("SELECT provider, COUNT(*) FROM agents GROUP BY provider ORDER BY provider")?;
    let linhas = consulta.query_map([], |linha| {
        let nome: String = linha.get(0)?;
        let total: i64 = linha.get(1)?;
        Ok((nome, total.max(0) as u64))
    })?;
    let mut contagem = Vec::new();
    for linha in linhas {
        contagem.push(linha?);
    }
    Ok(contagem)
}

/// Colunas do agente que podem recusar um valor (`CHECK`/`NOT NULL`) e o campo que a API expõe
/// para cada uma. Sem isto, a faixa de contexto apareceria como "campo slug inválido".
const CAMPOS_AGENTE: &[(&str, &str)] = &[
    ("name", "name"),
    ("status", "status"),
    ("agent_type", "type"),
    ("category", "category"),
    ("human_handoff_enabled", "handoff"),
    ("provider", "provider"),
    ("model", "model"),
    ("temperature", "temperature"),
    ("max_output_tokens", "max_output_tokens"),
    ("max_context_tokens", "max_context_tokens"),
    ("grouping_window_ms", "grouping_window_ms"),
    ("max_tool_rounds", "max_tool_rounds"),
    ("max_tool_calls_per_message", "max_tool_calls"),
    ("rag_top_k", "rag_top_k"),
    ("rag_min_score", "rag_min_score"),
];

const COLUNAS: &str = "id, org_id, name, slug, description, status, agent_type, category,
    default_department_id, human_handoff_enabled, human_handoff_department_id,
    provider, model, temperature, max_output_tokens,
    max_context_messages, max_context_tokens, grouping_window_ms,
    max_tool_rounds, max_tool_calls_per_message,
    rag_top_k, rag_min_score, jev_json, version, created_at, updated_at";

/// Acesso aos agentes.
pub struct Agentes<'a>(pub(crate) &'a Store);

impl Agentes<'_> {
    /// Cria o agente na org (status inicial `active`).
    pub fn criar(&self, org: OrgId, novo: NewAgent) -> Resultado<Agente> {
        let id = AgentId::generate();
        let agora = self.0.now();
        let texto_id = id.to_string();
        let texto_org = org.to_string();
        self.0.write(move |conn| {
            if let Some(departamento) = novo.default_department {
                pertence(
                    conn,
                    "departments",
                    "departamento",
                    &texto_org,
                    &departamento.to_string(),
                )?;
            }
            conn.execute(
                "INSERT INTO agents (
                    id, org_id, name, slug, description, status, agent_type, category,
                    default_department_id, human_handoff_enabled, human_handoff_department_id,
                    provider, model, temperature, max_output_tokens,
                    max_context_messages, max_context_tokens, grouping_window_ms,
                    max_tool_rounds, max_tool_calls_per_message,
                    rag_top_k, rag_min_score, jev_json, version, created_at, updated_at
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, 'active', ?6, ?7,
                    ?8, 0, NULL,
                    ?9, ?10, ?11, ?12,
                    30, ?13, ?14,
                    ?15, ?16,
                    ?17, ?18, '{}', 1, ?19, ?19
                 )",
                params![
                    texto_id,
                    texto_org,
                    novo.name,
                    novo.slug,
                    novo.description,
                    novo.agent_type,
                    novo.category,
                    novo.default_department.map(|dep| dep.to_string()),
                    novo.provedor,
                    novo.modelo,
                    novo.temperatura,
                    novo.max_tokens_saida,
                    novo.max_context_tokens,
                    novo.grouping_window_ms,
                    novo.max_tool_rounds,
                    novo.max_tools_per_message,
                    novo.rag_top_k,
                    novo.rag_cutoff,
                    agora
                ],
            )
            .map_err(|erro| StoreError::from_write_with("slug", CAMPOS_AGENTE, erro))?;
            Ok(())
        })?;
        self.get(org, id)
    }

    /// Busca pelo id dentro da org.
    pub fn get(&self, org: OrgId, id: AgentId) -> Resultado<Agente> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let encontrado = self.0.read(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUNAS} FROM agents WHERE id = ?1 AND org_id = ?2"),
                params![texto_id, texto_org],
                linha,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(encontrado, "agente", &id.to_string())
    }

    /// Busca pelo slug dentro da org.
    pub fn por_slug(&self, org: OrgId, slug: &str) -> Resultado<Agente> {
        let texto_org = org.to_string();
        let procurado = slug.to_owned();
        let rotulo = slug.to_owned();
        let encontrado = self.0.read(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUNAS} FROM agents WHERE org_id = ?1 AND slug = ?2"),
                params![texto_org, procurado],
                linha,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(encontrado, "agente", &format!("slug `{rotulo}`"))
    }

    /// Versão atual do agente, sem carregar o registro inteiro.
    ///
    /// É a consulta que a config quente (HR-11) usa para saber se precisa recarregar.
    pub fn versao(&self, org: OrgId, id: AgentId) -> Resultado<u64> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let encontrada = self.0.read(move |conn| {
            conn.query_row(
                "SELECT version FROM agents WHERE id = ?1 AND org_id = ?2",
                params![texto_id, texto_org],
                |linha| u64_da_coluna(linha, 0),
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(encontrada, "agente", &id.to_string())
    }

    /// Lista os agentes da org (cursor pelo id).
    pub fn listar(&self, org: OrgId, pagina: Page) -> Resultado<Vec<Agente>> {
        let texto_org = org.to_string();
        let apos = pagina.after.map(|id| id.to_string());
        let limite = i64::from(pagina.limit);
        self.0.read(move |conn| {
            let mut consulta = conn.prepare(&format!(
                "SELECT {COLUNAS} FROM agents
                  WHERE org_id = ?1 AND (?2 IS NULL OR id > ?2)
                  ORDER BY id LIMIT ?3"
            ))?;
            let linhas = consulta.query_map(params![texto_org, apos, limite], linha)?;
            let mut agentes = Vec::new();
            for linha in linhas {
                agentes.push(linha?);
            }
            Ok(agentes)
        })
    }

    /// Nome de cada agente pedido, numa consulta só (RP-01): a lista de conversas traz o
    /// `agent_name` da página inteira sem uma consulta por conversa. Agente que não existe nesta org
    /// fica de fora do resultado. Sempre filtrado por org.
    pub fn nomes(
        &self,
        org: OrgId,
        ids: &[AgentId],
    ) -> Resultado<std::collections::HashMap<AgentId, String>> {
        if ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let texto_org = org.to_string();
        let lista = serde_json::to_string(&ids.iter().map(ToString::to_string).collect::<Vec<_>>())
            .unwrap_or_else(|_| "[]".to_owned());
        self.0.read(move |conn| {
            let mut consulta = conn.prepare(
                "SELECT id, name FROM agents
                  WHERE org_id = ?1 AND id IN (SELECT value FROM json_each(?2))",
            )?;
            let linhas = consulta.query_map(params![texto_org, lista], |linha| {
                Ok((
                    id_da_coluna::<AgentId>(linha, 0)?,
                    linha.get::<_, String>(1)?,
                ))
            })?;
            let mut nomes = std::collections::HashMap::new();
            for linha in linhas {
                let (id, nome) = linha?;
                nomes.insert(id, nome);
            }
            Ok(nomes)
        })
    }

    /// Quantos agentes **ativos** cada org pedida tem, numa consulta só (CF-03a).
    ///
    /// É a coluna "agentes ativos" da listagem de orgs: uma consulta agregada para a página
    /// inteira, nunca uma por linha (a mesma regra do RP-01). Org sem agente ativo **não aparece**
    /// no mapa — quem chama trata a ausência como zero.
    pub fn contar_ativos_por_org(&self, orgs: &[OrgId]) -> Resultado<HashMap<OrgId, u32>> {
        if orgs.is_empty() {
            return Ok(HashMap::new());
        }
        let lista =
            serde_json::to_string(&orgs.iter().map(ToString::to_string).collect::<Vec<_>>())
                .unwrap_or_else(|_| "[]".to_owned());
        self.0.read(move |conn| {
            let mut consulta = conn.prepare(
                "SELECT org_id, count(*) FROM agents
                  WHERE status = 'active' AND org_id IN (SELECT value FROM json_each(?1))
                  GROUP BY org_id",
            )?;
            let linhas = consulta.query_map(params![lista], |linha| {
                Ok((id_da_coluna::<OrgId>(linha, 0)?, u32_da_coluna(linha, 1)?))
            })?;
            let mut contagem = HashMap::new();
            for linha in linhas {
                let (org, total) = linha?;
                contagem.insert(org, total);
            }
            Ok(contagem)
        })
    }

    /// Busca o agente **para abrir turno**, aplicando a decisão D-05.
    ///
    /// É o método que o motor do turno (HR-13) usa: agente inexistente devolve "não encontrado";
    /// agente `inactive` ou `archived` devolve [`poph_types::DomainError::AgenteInativo`], nunca um
    /// agente pronto para responder. A mensagem recebida fica registrada e é descartada sem
    /// resposta (o registro fica em `audit_log`/`turns`, na HR-13).
    pub fn obter_para_turno(&self, org: OrgId, id: AgentId) -> Resultado<Agente> {
        let agente = self.get(org, id)?;
        if !agente.can_open_turn() {
            return Err(StoreError::Domain(poph_types::DomainError::AgentInactive {
                id: agente.id.to_string(),
                status: agente.status.as_text().to_owned(),
            }));
        }
        Ok(agente)
    }

    /// Lista só os agentes que podem abrir turno (decisão D-05: `active`).
    pub fn listar_ativos(&self, org: OrgId) -> Resultado<Vec<Agente>> {
        let texto_org = org.to_string();
        self.0.read(move |conn| {
            let mut consulta = conn.prepare(&format!(
                "SELECT {COLUNAS} FROM agents WHERE org_id = ?1 AND status = 'active' ORDER BY id"
            ))?;
            let linhas = consulta.query_map(params![texto_org], linha)?;
            let mut agentes = Vec::new();
            for linha in linhas {
                agentes.push(linha?);
            }
            Ok(agentes)
        })
    }

    /// Altera com `If-Match`. Campo `None` fica como está (não zera).
    pub fn atualizar(
        &self,
        org: OrgId,
        id: AgentId,
        esperada: u64,
        mudanca: AgentChange,
    ) -> Resultado<Agente> {
        let agora = self.0.now();
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let status = mudanca.status.map(Status::as_text);
        let handoff_ativo = mudanca.handoff_active.map(i64::from);
        let esperada_i64 = esperada as i64;
        let efeito = self.0.write(move |conn| {
            for departamento in [mudanca.default_department, mudanca.handoff_department]
                .into_iter()
                .flatten()
            {
                pertence(
                    conn,
                    "departments",
                    "departamento",
                    &texto_org,
                    &departamento.to_string(),
                )?;
            }
            conn.execute(
                "UPDATE agents SET
                    name = COALESCE(:nome, name),
                    description = COALESCE(:descricao, description),
                    status = COALESCE(:status, status),
                    default_department_id = COALESCE(:departamento_padrao, default_department_id),
                    human_handoff_enabled = COALESCE(:handoff_ativo, human_handoff_enabled),
                    human_handoff_department_id = COALESCE(:handoff_departamento, human_handoff_department_id),
                    provider = COALESCE(:provedor, provider),
                    model = COALESCE(:modelo, model),
                    temperature = COALESCE(:temperatura, temperature),
                    max_output_tokens = COALESCE(:max_tokens_saida, max_output_tokens),
                    max_context_tokens = COALESCE(:max_context_tokens, max_context_tokens),
                    grouping_window_ms = COALESCE(:janela, grouping_window_ms),
                    max_tool_rounds = COALESCE(:rodadas, max_tool_rounds),
                    max_tool_calls_per_message = COALESCE(:calls, max_tool_calls_per_message),
                    rag_top_k = COALESCE(:top_k, rag_top_k),
                    rag_min_score = COALESCE(:corte, rag_min_score),
                    jev_json = COALESCE(:jev, jev_json),
                    version = version + 1,
                    updated_at = :agora
                  WHERE id = :id AND org_id = :org AND version = :esperada",
                named_params! {
                    ":nome": mudanca.name,
                    ":descricao": mudanca.description,
                    ":status": status,
                    ":departamento_padrao": mudanca.default_department.map(|dep| dep.to_string()),
                    ":handoff_ativo": handoff_ativo,
                    ":handoff_departamento": mudanca.handoff_department.map(|dep| dep.to_string()),
                    ":provedor": mudanca.provedor,
                    ":modelo": mudanca.modelo,
                    ":temperatura": mudanca.temperatura,
                    ":max_tokens_saida": mudanca.max_tokens_saida,
                    ":max_context_tokens": mudanca.max_context_tokens.map(i64::from),
                    ":janela": mudanca.grouping_window_ms.map(i64::from),
                    ":rodadas": mudanca.max_tool_rounds.map(i64::from),
                    ":calls": mudanca.max_tools_per_message.map(i64::from),
                    ":top_k": mudanca.rag_top_k.map(i64::from),
                    ":corte": mudanca.rag_cutoff,
                    ":jev": mudanca.jev_json,
                    ":agora": agora,
                    ":id": texto_id,
                    ":org": texto_org,
                    ":esperada": esperada_i64,
                },
            )
            .map_err(|erro| StoreError::from_write_with("slug", CAMPOS_AGENTE, erro))
        })?;
        if efeito == 0 {
            return Err(self.conflito(org, id, esperada));
        }
        self.get(org, id)
    }

    /// Exclui o agente (instruções, vínculos, conversas e arquivos dele saem junto).
    pub fn excluir(&self, org: OrgId, id: AgentId) -> Resultado<()> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let apagou = self.0.write(move |conn| {
            conn.execute(
                "DELETE FROM agents WHERE org_id = ?1 AND id = ?2",
                params![texto_org, texto_id],
            )
            .map_err(StoreError::from)
        })?;
        if apagou == 0 {
            return Err(nao_encontrado(id));
        }
        Ok(())
    }

    /// Substitui as categorias de produto do agente, com `If-Match` na versão do agente.
    pub fn definir_categorias(
        &self,
        org: OrgId,
        id: AgentId,
        esperada: u64,
        categorias: &[ProductCategoryId],
    ) -> Resultado<Agente> {
        let agora = self.0.now();
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let categorias: Vec<String> = categorias.iter().map(ToString::to_string).collect();
        let esperada_i64 = esperada as i64;
        self.0.write(move |conn| {
            for categoria in &categorias {
                pertence(
                    conn,
                    "product_categories",
                    "categoria de produto",
                    &texto_org,
                    categoria,
                )?;
            }
            let efeito = conn
                .execute(
                    "UPDATE agents SET version = version + 1, updated_at = ?3
                      WHERE id = ?1 AND org_id = ?2 AND version = ?4",
                    params![texto_id, texto_org, agora, esperada_i64],
                )
                .map_err(StoreError::from)?;
            if efeito == 0 {
                return Err(conflito_ou_ausente(
                    conn, "agents", "agente", &texto_id, &texto_org, esperada,
                ));
            }
            conn.execute(
                "DELETE FROM agent_product_categories WHERE org_id = ?1 AND agent_id = ?2",
                params![texto_org, texto_id],
            )
            .map_err(StoreError::from)?;
            for categoria in &categorias {
                conn.execute(
                    "INSERT INTO agent_product_categories (org_id, agent_id, category_id)
                     VALUES (?1, ?2, ?3)",
                    params![texto_org, texto_id, categoria],
                )
                .map_err(StoreError::from)?;
            }
            Ok(())
        })?;
        self.get(org, id)
    }

    /// Categorias de produto vinculadas ao agente.
    pub fn categorias(&self, org: OrgId, id: AgentId) -> Resultado<Vec<ProductCategoryId>> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        self.0.read(move |conn| {
            let mut consulta = conn.prepare(
                "SELECT category_id FROM agent_product_categories
                  WHERE org_id = ?1 AND agent_id = ?2 ORDER BY category_id",
            )?;
            let linhas = consulta.query_map(params![texto_org, texto_id], |linha| {
                id_da_coluna::<ProductCategoryId>(linha, 0)
            })?;
            let mut categorias = Vec::new();
            for linha in linhas {
                categorias.push(linha?);
            }
            Ok(categorias)
        })
    }

    fn conflito(&self, org: OrgId, id: AgentId, esperada: u64) -> StoreError {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        self.0
            .read(move |conn| {
                Ok(conflito_ou_ausente(
                    conn, "agents", "agente", &texto_id, &texto_org, esperada,
                ))
            })
            .unwrap_or_else(|erro| erro)
    }
}

fn nao_encontrado(id: AgentId) -> StoreError {
    StoreError::Domain(poph_types::DomainError::NotFound {
        entity: "agente",
        id: id.to_string(),
    })
}

fn linha(linha: &Row<'_>) -> rusqlite::Result<Agente> {
    Ok(Agente {
        id: id_da_coluna(linha, 0)?,
        org_id: id_da_coluna(linha, 1)?,
        name: linha.get(2)?,
        slug: linha.get(3)?,
        description: linha.get(4)?,
        status: status_da_coluna(linha, 5)?,
        agent_type: linha.get(6)?,
        category: linha.get(7)?,
        default_department: id_opcional(linha, 8)?,
        handoff_active: linha.get::<_, i64>(9)? != 0,
        handoff_department: id_opcional(linha, 10)?,
        provedor: linha.get(11)?,
        modelo: linha.get(12)?,
        temperatura: linha.get(13)?,
        max_tokens_saida: linha.get(14)?,
        max_context_messages: u32_da_coluna(linha, 15)?,
        max_context_tokens: u32_da_coluna(linha, 16)?,
        grouping_window_ms: u32_da_coluna(linha, 17)?,
        max_tool_rounds: u32_da_coluna(linha, 18)?,
        max_tools_per_message: u32_da_coluna(linha, 19)?,
        rag_top_k: u32_da_coluna(linha, 20)?,
        rag_cutoff: linha.get(21)?,
        jev_json: linha.get(22)?,
        version: u64_da_coluna(linha, 23)?,
        created_at: linha.get(24)?,
        updated_at: linha.get(25)?,
    })
}
