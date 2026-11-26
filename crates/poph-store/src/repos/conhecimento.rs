//! Base de conhecimento: grupos, itens e o vínculo grupo↔agente (HR-17).
//!
//! Regras que este repositório garante:
//! - **`org_id` em toda assinatura.** Grupo e item são lidos e escritos sempre dentro da org; o
//!   vínculo com agente exige que os dois sejam da mesma org (chave estrangeira composta).
//! - **Limite de 1000 caracteres** do conteúdo (PARIDADE_AGENTE.md §3). O banco tem o `CHECK`; aqui
//!   a validação existe para o erro dizer qual campo e quantos caracteres vieram.
//! - **D-05**: [`Conhecimento::itens_do_agente`] devolve só item ativo de grupo ativo. Item ou grupo
//!   inativo não entra no índice e não volta da busca.
//! - **Versão para `If-Match`** nas duas entidades editáveis.

use poph_types::{AgentId, KnowledgeGroupId, KnowledgeItemId, OrgId};
use rusqlite::{OptionalExtension, Row, params};

use crate::Store;
use crate::erro::{ErroStore, Resultado};
use crate::modelo::{
    GrupoConhecimento, ItemConhecimento, ItemParaIndice, LIMITE_CONTEUDO_ITEM,
    MudancaGrupoConhecimento, MudancaItemConhecimento, NovoGrupoConhecimento, NovoItemConhecimento,
    Pagina, Status,
};
use crate::repos::{conflito_ou_ausente, id_da_coluna, pertence, status_da_coluna, u64_da_coluna};

const COLUNAS_GRUPO: &str =
    "id, org_id, name, description, status, version, created_at, updated_at";
/// Colunas do item que podem recusar um valor e o campo que a API expõe para cada uma.
const CAMPOS_ITEM: &[(&str, &str)] = &[
    ("title", "título do item"),
    ("content", "conteúdo do item"),
    ("status", "status do item"),
];

const COLUNAS_ITEM: &str = "id, org_id, group_id, title, content, tags_json, position, status,
    version, created_at, updated_at";

/// Acesso à base de conhecimento.
pub struct Conhecimento<'a>(pub(crate) &'a Store);

impl Conhecimento<'_> {
    // ───────────────────────────── Grupos ─────────────────────────────

    /// Cria o grupo na org.
    pub fn criar_grupo(
        &self,
        org: OrgId,
        novo: NovoGrupoConhecimento,
    ) -> Resultado<GrupoConhecimento> {
        let nome = validar_nome("nome do grupo", &novo.nome, 120)?;
        let id = KnowledgeGroupId::generate();
        let agora = self.0.agora();
        let texto_id = id.to_string();
        let texto_org = org.to_string();
        self.0.escrita(move |conn| {
            conn.execute(
                "INSERT INTO knowledge_groups
                    (id, org_id, name, description, status, version, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 'active', 1, ?5, ?5)",
                params![texto_id, texto_org, nome, novo.descricao.trim(), agora],
            )
            .map_err(|erro| ErroStore::da_escrita("nome do grupo", erro))?;
            Ok(())
        })?;
        self.obter_grupo(org, id)
    }

    /// Busca o grupo pelo id dentro da org.
    pub fn obter_grupo(&self, org: OrgId, id: KnowledgeGroupId) -> Resultado<GrupoConhecimento> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let encontrado = self.0.leitura(move |conn| {
            conn.query_row(
                &format!(
                    "SELECT {COLUNAS_GRUPO} FROM knowledge_groups WHERE id = ?1 AND org_id = ?2"
                ),
                params![texto_id, texto_org],
                linha_grupo,
            )
            .optional()
            .map_err(ErroStore::from)
        })?;
        super::exigir(encontrado, "grupo de conhecimento", &id.to_string())
    }

    /// Lista os grupos da org.
    pub fn listar_grupos(&self, org: OrgId, pagina: Pagina) -> Resultado<Vec<GrupoConhecimento>> {
        let texto_org = org.to_string();
        let apos = pagina.apos.map(|id| id.to_string());
        let limite = i64::from(pagina.limite);
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(&format!(
                "SELECT {COLUNAS_GRUPO} FROM knowledge_groups
                  WHERE org_id = ?1 AND (?2 IS NULL OR id > ?2)
                  ORDER BY id LIMIT ?3"
            ))?;
            let linhas = consulta.query_map(params![texto_org, apos, limite], linha_grupo)?;
            let mut grupos = Vec::new();
            for linha in linhas {
                grupos.push(linha?);
            }
            Ok(grupos)
        })
    }

    /// Altera o grupo com `If-Match`.
    pub fn atualizar_grupo(
        &self,
        org: OrgId,
        id: KnowledgeGroupId,
        esperada: u64,
        mudanca: MudancaGrupoConhecimento,
    ) -> Resultado<GrupoConhecimento> {
        let nome = match &mudanca.nome {
            Some(nome) => Some(validar_nome("nome do grupo", nome, 120)?),
            None => None,
        };
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let status = mudanca.status.map(Status::como_texto);
        let esperada_i64 = esperada as i64;
        let efeito = self.0.escrita(move |conn| {
            conn.execute(
                "UPDATE knowledge_groups
                    SET name = COALESCE(?3, name),
                        description = COALESCE(?4, description),
                        status = COALESCE(?5, status),
                        version = version + 1,
                        updated_at = ?6
                  WHERE org_id = ?1 AND id = ?2 AND version = ?7",
                params![
                    texto_org,
                    texto_id,
                    nome,
                    mudanca.descricao.map(|texto| texto.trim().to_owned()),
                    status,
                    agora,
                    esperada_i64
                ],
            )
            .map_err(|erro| ErroStore::da_escrita("nome do grupo", erro))
        })?;
        if efeito == 0 {
            return Err(self.conflito_grupo(org, id, esperada));
        }
        self.obter_grupo(org, id)
    }

    /// Exclui o grupo (itens e vínculos com agentes saem junto, em cascata).
    pub fn excluir_grupo(&self, org: OrgId, id: KnowledgeGroupId) -> Resultado<()> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let apagou = self.0.escrita(move |conn| {
            conn.execute(
                "DELETE FROM knowledge_groups WHERE org_id = ?1 AND id = ?2",
                params![texto_org, texto_id],
            )
            .map_err(ErroStore::from)
        })?;
        if apagou == 0 {
            return Err(nao_encontrado("grupo de conhecimento", &id.to_string()));
        }
        Ok(())
    }

    /// Lista itens por id para paginação por cursor estável.
    pub fn listar_itens_por_id(
        &self,
        org: OrgId,
        grupo: KnowledgeGroupId,
        pagina: Pagina,
    ) -> Resultado<Vec<ItemConhecimento>> {
        let texto_org = org.to_string();
        let texto_grupo = grupo.to_string();
        let apos = pagina.apos.map(|id| id.to_string());
        let limite = i64::from(pagina.limite);
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(&format!(
                "SELECT {COLUNAS_ITEM} FROM knowledge_items
                 WHERE org_id = ?1 AND group_id = ?2 AND (?3 IS NULL OR id > ?3)
                 ORDER BY id LIMIT ?4"
            ))?;
            let linhas =
                consulta.query_map(params![texto_org, texto_grupo, apos, limite], linha_item)?;
            let mut itens = Vec::new();
            for linha in linhas {
                itens.push(linha?);
            }
            Ok(itens)
        })
    }

    // ───────────────────────────── Itens ─────────────────────────────

    /// Cria o item no grupo, dentro da org.
    pub fn criar_item(
        &self,
        org: OrgId,
        novo: NovoItemConhecimento,
    ) -> Resultado<ItemConhecimento> {
        let titulo = validar_nome("título do item", &novo.titulo, 200)?;
        let conteudo = validar_conteudo(&novo.conteudo)?;
        let tags = serde_json::to_string(&novo.tags).unwrap_or_else(|_| "[]".to_owned());
        let id = KnowledgeItemId::generate();
        let agora = self.0.agora();
        let texto_id = id.to_string();
        let texto_org = org.to_string();
        let grupo = novo.grupo_id.to_string();
        self.0.escrita(move |conn| {
            pertence(
                conn,
                "knowledge_groups",
                "grupo de conhecimento",
                &texto_org,
                &grupo,
            )?;
            conn.execute(
                "INSERT INTO knowledge_items
                    (id, org_id, group_id, title, content, tags_json, source_type, position, status,
                     version, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'manual', ?7, 'active', 1, ?8, ?8)",
                params![
                    texto_id,
                    texto_org,
                    grupo,
                    titulo,
                    conteudo,
                    tags,
                    novo.posicao,
                    agora
                ],
            )
            .map_err(|erro| ErroStore::da_escrita_em("título do item", CAMPOS_ITEM, erro))?;
            Ok(())
        })?;
        self.obter_item(org, id)
    }

    /// Busca o item pelo id dentro da org.
    pub fn obter_item(&self, org: OrgId, id: KnowledgeItemId) -> Resultado<ItemConhecimento> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let encontrado = self.0.leitura(move |conn| {
            conn.query_row(
                &format!(
                    "SELECT {COLUNAS_ITEM} FROM knowledge_items WHERE id = ?1 AND org_id = ?2"
                ),
                params![texto_id, texto_org],
                linha_item,
            )
            .optional()
            .map_err(ErroStore::from)
        })?;
        super::exigir(encontrado, "item de conhecimento", &id.to_string())
    }

    /// Lista os itens do grupo (todas as situações: a CLI precisa ver o inativo também).
    pub fn listar_itens(
        &self,
        org: OrgId,
        grupo: KnowledgeGroupId,
        pagina: Pagina,
    ) -> Resultado<Vec<ItemConhecimento>> {
        let texto_org = org.to_string();
        let texto_grupo = grupo.to_string();
        let apos = pagina.apos.map(|id| id.to_string());
        let limite = i64::from(pagina.limite);
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(&format!(
                "SELECT {COLUNAS_ITEM} FROM knowledge_items
                  WHERE org_id = ?1 AND group_id = ?2 AND (?3 IS NULL OR id > ?3)
                  ORDER BY position, id LIMIT ?4"
            ))?;
            let linhas =
                consulta.query_map(params![texto_org, texto_grupo, apos, limite], linha_item)?;
            let mut itens = Vec::new();
            for linha in linhas {
                itens.push(linha?);
            }
            Ok(itens)
        })
    }

    /// Altera o item com `If-Match`.
    pub fn atualizar_item(
        &self,
        org: OrgId,
        id: KnowledgeItemId,
        esperada: u64,
        mudanca: MudancaItemConhecimento,
    ) -> Resultado<ItemConhecimento> {
        let titulo = match &mudanca.titulo {
            Some(titulo) => Some(validar_nome("título do item", titulo, 200)?),
            None => None,
        };
        let conteudo = match &mudanca.conteudo {
            Some(conteudo) => Some(validar_conteudo(conteudo)?),
            None => None,
        };
        let tags = mudanca
            .tags
            .as_ref()
            .map(|tags| serde_json::to_string(tags).unwrap_or_else(|_| "[]".to_owned()));
        let grupo = mudanca.grupo_id.map(|grupo| grupo.to_string());
        let status = mudanca.status.map(Status::como_texto);
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let esperada_i64 = esperada as i64;
        let efeito = self.0.escrita(move |conn| {
            if let Some(grupo) = &grupo {
                pertence(
                    conn,
                    "knowledge_groups",
                    "grupo de conhecimento",
                    &texto_org,
                    grupo,
                )?;
            }
            conn.execute(
                "UPDATE knowledge_items
                    SET group_id = COALESCE(?3, group_id),
                        title = COALESCE(?4, title),
                        content = COALESCE(?5, content),
                        tags_json = COALESCE(?6, tags_json),
                        position = COALESCE(?7, position),
                        status = COALESCE(?8, status),
                        version = version + 1,
                        updated_at = ?9
                  WHERE org_id = ?1 AND id = ?2 AND version = ?10",
                params![
                    texto_org,
                    texto_id,
                    grupo,
                    titulo,
                    conteudo,
                    tags,
                    mudanca.posicao,
                    status,
                    agora,
                    esperada_i64
                ],
            )
            .map_err(|erro| ErroStore::da_escrita_em("título do item", CAMPOS_ITEM, erro))
        })?;
        if efeito == 0 {
            return Err(self.conflito_item(org, id, esperada));
        }
        self.obter_item(org, id)
    }

    /// Exclui o item.
    pub fn excluir_item(&self, org: OrgId, id: KnowledgeItemId) -> Resultado<()> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let apagou = self.0.escrita(move |conn| {
            conn.execute(
                "DELETE FROM knowledge_items WHERE org_id = ?1 AND id = ?2",
                params![texto_org, texto_id],
            )
            .map_err(ErroStore::from)
        })?;
        if apagou == 0 {
            return Err(nao_encontrado("item de conhecimento", &id.to_string()));
        }
        Ok(())
    }

    // ───────────────────────────── Vínculo com agentes ─────────────────────────────

    /// Liga o grupo ao agente (idempotente: vincular de novo não é erro).
    pub fn vincular_agente(
        &self,
        org: OrgId,
        agente: AgentId,
        grupo: KnowledgeGroupId,
    ) -> Resultado<()> {
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let texto_agente = agente.to_string();
        let texto_grupo = grupo.to_string();
        self.0.escrita(move |conn| {
            pertence(conn, "agents", "agente", &texto_org, &texto_agente)?;
            pertence(
                conn,
                "knowledge_groups",
                "grupo de conhecimento",
                &texto_org,
                &texto_grupo,
            )?;
            conn.execute(
                "INSERT INTO agent_knowledge_groups (org_id, agent_id, group_id, version, created_at)
                 VALUES (?1, ?2, ?3, 1, ?4)
                 ON CONFLICT (agent_id, group_id) DO NOTHING",
                params![texto_org, texto_agente, texto_grupo, agora],
            )
            .map_err(ErroStore::from)?;
            Ok(())
        })?;
        Ok(())
    }

    /// Desliga o grupo do agente. Devolve `true` quando havia vínculo.
    pub fn desvincular_agente(
        &self,
        org: OrgId,
        agente: AgentId,
        grupo: KnowledgeGroupId,
    ) -> Resultado<bool> {
        let texto_org = org.to_string();
        let texto_agente = agente.to_string();
        let texto_grupo = grupo.to_string();
        let apagou = self.0.escrita(move |conn| {
            conn.execute(
                "DELETE FROM agent_knowledge_groups
                  WHERE org_id = ?1 AND agent_id = ?2 AND group_id = ?3",
                params![texto_org, texto_agente, texto_grupo],
            )
            .map_err(ErroStore::from)
        })?;
        Ok(apagou > 0)
    }

    /// Grupos ligados ao agente, em ordem de nome.
    pub fn grupos_do_agente(
        &self,
        org: OrgId,
        agente: AgentId,
    ) -> Resultado<Vec<GrupoConhecimento>> {
        let texto_org = org.to_string();
        let texto_agente = agente.to_string();
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(&format!(
                "SELECT {} FROM knowledge_groups g
                   JOIN agent_knowledge_groups v
                     ON v.org_id = g.org_id AND v.group_id = g.id
                  WHERE g.org_id = ?1 AND v.agent_id = ?2
                  ORDER BY lower(g.name), g.id",
                prefixar("g", COLUNAS_GRUPO)
            ))?;
            let linhas = consulta.query_map(params![texto_org, texto_agente], linha_grupo)?;
            let mut grupos = Vec::new();
            for linha in linhas {
                grupos.push(linha?);
            }
            Ok(grupos)
        })
    }

    /// Agentes que usam o grupo, em ordem de nome.
    pub fn agentes_do_grupo(&self, org: OrgId, grupo: KnowledgeGroupId) -> Resultado<Vec<AgentId>> {
        let texto_org = org.to_string();
        let texto_grupo = grupo.to_string();
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(
                "SELECT agent_id FROM agent_knowledge_groups
                  WHERE org_id = ?1 AND group_id = ?2 ORDER BY agent_id",
            )?;
            let linhas = consulta.query_map(params![texto_org, texto_grupo], |linha| {
                id_da_coluna::<AgentId>(linha, 0)
            })?;
            let mut agentes = Vec::new();
            for linha in linhas {
                agentes.push(linha?);
            }
            Ok(agentes)
        })
    }

    /// Itens que entram no índice do agente: **só item ativo de grupo ativo** (D-05), dos grupos
    /// ligados a ele. Traz o nome do grupo para o registro do turno.
    pub fn itens_do_agente(&self, org: OrgId, agente: AgentId) -> Resultado<Vec<ItemParaIndice>> {
        let texto_org = org.to_string();
        let texto_agente = agente.to_string();
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(
                "SELECT i.id, i.group_id, g.name, i.title, i.content, i.version
                   FROM knowledge_items i
                   JOIN knowledge_groups g
                     ON g.org_id = i.org_id AND g.id = i.group_id
                   JOIN agent_knowledge_groups v
                     ON v.org_id = g.org_id AND v.group_id = g.id
                  WHERE i.org_id = ?1 AND v.agent_id = ?2
                    AND i.status = 'active' AND g.status = 'active'
                  ORDER BY lower(g.name), i.position, i.id",
            )?;
            let linhas = consulta.query_map(params![texto_org, texto_agente], |linha| {
                Ok(ItemParaIndice {
                    id: id_da_coluna(linha, 0)?,
                    grupo_id: id_da_coluna(linha, 1)?,
                    grupo: linha.get(2)?,
                    titulo: linha.get(3)?,
                    conteudo: linha.get(4)?,
                    versao: u64_da_coluna(linha, 5)?,
                })
            })?;
            let mut itens = Vec::new();
            for linha in linhas {
                itens.push(linha?);
            }
            Ok(itens)
        })
    }

    /// Nome do grupo de um item (fonte mostrada ao modelo). `None` quando o item não existe na org.
    pub fn nome_do_grupo_do_item(
        &self,
        org: OrgId,
        item: KnowledgeItemId,
    ) -> Resultado<Option<String>> {
        let texto_org = org.to_string();
        let texto_item = item.to_string();
        self.0.leitura(move |conn| {
            conn.query_row(
                "SELECT g.name FROM knowledge_items i
                   JOIN knowledge_groups g ON g.org_id = i.org_id AND g.id = i.group_id
                  WHERE i.org_id = ?1 AND i.id = ?2",
                params![texto_org, texto_item],
                |linha| linha.get(0),
            )
            .optional()
            .map_err(ErroStore::from)
        })
    }

    fn conflito_grupo(&self, org: OrgId, id: KnowledgeGroupId, esperada: u64) -> ErroStore {
        self.0
            .leitura(move |conn| {
                Ok(conflito_ou_ausente(
                    conn,
                    "knowledge_groups",
                    "grupo de conhecimento",
                    &id.to_string(),
                    &org.to_string(),
                    esperada,
                ))
            })
            .unwrap_or_else(|erro| erro)
    }

    fn conflito_item(&self, org: OrgId, id: KnowledgeItemId, esperada: u64) -> ErroStore {
        self.0
            .leitura(move |conn| {
                Ok(conflito_ou_ausente(
                    conn,
                    "knowledge_items",
                    "item de conhecimento",
                    &id.to_string(),
                    &org.to_string(),
                    esperada,
                ))
            })
            .unwrap_or_else(|erro| erro)
    }
}

/// Prefixa as colunas de uma lista "a, b, c" com o apelido da tabela ("g.a, g.b, g.c").
fn prefixar(apelido: &str, colunas: &str) -> String {
    colunas
        .split(',')
        .map(|coluna| format!("{apelido}.{}", coluna.trim()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn validar_nome(campo: &'static str, texto: &str, limite: usize) -> Resultado<String> {
    let limpo = texto.trim();
    if limpo.is_empty() {
        return Err(ErroStore::Dominio(poph_types::DomainError::Validation {
            field: campo,
            reason: "não pode ficar em branco".to_owned(),
        }));
    }
    if limpo.chars().count() > limite {
        return Err(ErroStore::Dominio(poph_types::DomainError::Validation {
            field: campo,
            reason: format!("até {limite} caracteres; vieram {}", limpo.chars().count()),
        }));
    }
    Ok(limpo.to_owned())
}

fn validar_conteudo(texto: &str) -> Resultado<String> {
    let limpo = texto.trim();
    let caracteres = limpo.chars().count();
    if caracteres == 0 {
        return Err(ErroStore::Dominio(poph_types::DomainError::Validation {
            field: "conteúdo do item",
            reason: "não pode ficar em branco".to_owned(),
        }));
    }
    if caracteres > LIMITE_CONTEUDO_ITEM {
        return Err(ErroStore::Dominio(poph_types::DomainError::Validation {
            field: "conteúdo do item",
            reason: format!("até {LIMITE_CONTEUDO_ITEM} caracteres; vieram {caracteres}"),
        }));
    }
    Ok(limpo.to_owned())
}

fn nao_encontrado(entidade: &'static str, id: &str) -> ErroStore {
    ErroStore::Dominio(poph_types::DomainError::NotFound {
        entity: entidade,
        id: id.to_owned(),
    })
}

fn linha_grupo(linha: &Row<'_>) -> rusqlite::Result<GrupoConhecimento> {
    Ok(GrupoConhecimento {
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

fn linha_item(linha: &Row<'_>) -> rusqlite::Result<ItemConhecimento> {
    let tags: String = linha.get(5)?;
    Ok(ItemConhecimento {
        id: id_da_coluna(linha, 0)?,
        org_id: id_da_coluna(linha, 1)?,
        grupo_id: id_da_coluna(linha, 2)?,
        titulo: linha.get(3)?,
        conteudo: linha.get(4)?,
        tags: serde_json::from_str(&tags).unwrap_or_default(),
        posicao: linha.get(6)?,
        status: status_da_coluna(linha, 7)?,
        versao: u64_da_coluna(linha, 8)?,
        criado_em: linha.get(9)?,
        atualizado_em: linha.get(10)?,
    })
}
