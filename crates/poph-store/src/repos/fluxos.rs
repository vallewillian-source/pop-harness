//! Repositório de fluxos e da permissão por agente (FX-08, PRD §6.2 e §7.1).
//!
//! O fluxo é da organização; o agente só usa o que estiver em `agente_fluxos` (padrão negado). Este
//! arquivo guarda o cadastro, o rascunho, a permissão e as regras de teto, de organização e de
//! exclusão. Não compila script e não publica revisão: a publicação é a FX-09
//! (`repos::revisoes::Revisoes::publicar_revisao`) e o compilador, a FX-10.
//!
//! Três regras que o repositório garante, além do que o banco garante:
//!
//! - **teto de [`TETO_FLUXOS_POR_AGENTE`] fluxos por agente**: a permissão que estoura é recusada
//!   inteira, com [`DomainError::TetoDeFluxosPorAgente`];
//! - **fluxo e agente da mesma org**: permissão que cite registro de outra org (ou inexistente) é
//!   recusada com `NaoEncontrado`, não com erro de chave estrangeira;
//! - **permissão muda a versão do agente**: liberar, revogar e substituir gravam o `version` do
//!   agente, para que quem leu a lista com a versão antiga leve `412` no `If-Match` seguinte e não
//!   sobrescreva a mudança de outra pessoa.

use poph_types::{AgentId, AtorEvento, DomainError, ExecucaoId, FluxoId, OrgId, TipoEventoFicha};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::fichas::registrar_evento_conn;
use super::{conflito_ou_ausente, id_da_coluna, pertence, status_da_coluna, u64_da_coluna};
use crate::Store;
use crate::erro::{ErroStore, Resultado};
use crate::modelo::{
    FiltroFluxos, Fluxo, MudancaFluxo, NovoEventoFicha, NovoFluxo, Pagina, Status,
};

/// Quantos fluxos um agente pode ter liberados ao mesmo tempo (PRD §15: o G9 do Jev precisa de
/// opções finitas). É do repositório: quem libera ou substitui a lista não passa dele.
pub const TETO_FLUXOS_POR_AGENTE: u32 = 20;

/// Motivo gravado nas execuções abandonadas quando o fluxo é excluído com `force` (PRD §6.2).
pub const MOTIVO_EXCLUSAO_FLUXO: &str = "fluxo_excluido";

const COLUNAS: &str = "id, org_id, slug, nome, descricao, status, rascunho_fonte, revisao_atual,
    version";

/// Acesso aos fluxos e à permissão de cada agente.
pub struct Fluxos<'a>(pub(crate) &'a Store);

impl Fluxos<'_> {
    /// Cria o fluxo na org, com situação `active` e sem revisão publicada.
    pub fn criar(&self, org: OrgId, novo: NovoFluxo) -> Resultado<Fluxo> {
        validar_slug(&novo.slug)?;
        validar_nome(&novo.nome)?;
        let id = FluxoId::generate();
        let texto_id = id.to_string();
        let texto_org = org.to_string();
        self.0.escrita(move |conn| {
            conn.execute(
                "INSERT INTO fluxos
                    (id, org_id, slug, nome, descricao, status, rascunho_fonte, revisao_atual, version)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, NULL, 1)",
                params![
                    texto_id,
                    texto_org,
                    novo.slug,
                    novo.nome,
                    novo.descricao,
                    novo.rascunho_fonte
                ],
            )
            .map_err(|erro| {
                ErroStore::da_escrita_em(
                    "slug",
                    &[("nome", "nome"), ("descricao", "descricao")],
                    erro,
                )
            })?;
            Ok(())
        })?;
        self.obter(org, id)
    }

    /// Busca pelo id dentro da org.
    pub fn obter(&self, org: OrgId, id: FluxoId) -> Resultado<Fluxo> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let encontrado = self.0.leitura(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUNAS} FROM fluxos WHERE org_id = ?1 AND id = ?2"),
                params![texto_org, texto_id],
                linha,
            )
            .optional()
            .map_err(ErroStore::from)
        })?;
        super::exigir(encontrado, "fluxo", &id.to_string())
    }

    /// Fluxo pelo slug (identificador do script) dentro da org, se houver.
    pub fn por_slug(&self, org: OrgId, slug: &str) -> Resultado<Option<Fluxo>> {
        let texto_org = org.to_string();
        let texto_slug = slug.to_owned();
        self.0.leitura(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUNAS} FROM fluxos WHERE org_id = ?1 AND slug = ?2"),
                params![texto_org, texto_slug],
                linha,
            )
            .optional()
            .map_err(ErroStore::from)
        })
    }

    /// Lista os fluxos da org com filtro de situação, em ordem de id e paginada pelo cursor.
    pub fn listar(
        &self,
        org: OrgId,
        filtro: &FiltroFluxos,
        pagina: Pagina,
    ) -> Resultado<Vec<Fluxo>> {
        let mut condicoes = vec!["org_id = ?1".to_owned()];
        let mut valores: Vec<rusqlite::types::Value> =
            vec![rusqlite::types::Value::Text(org.to_string())];
        if let Some(status) = filtro.status {
            valores.push(rusqlite::types::Value::Text(status.como_texto().to_owned()));
            condicoes.push(format!("status = ?{}", valores.len()));
        }
        if let Some(apos) = pagina.apos {
            valores.push(rusqlite::types::Value::Text(apos.to_string()));
            condicoes.push(format!("id > ?{}", valores.len()));
        }
        valores.push(rusqlite::types::Value::Integer(i64::from(pagina.limite)));
        let sql = format!(
            "SELECT {COLUNAS} FROM fluxos WHERE {} ORDER BY id LIMIT ?{}",
            condicoes.join(" AND "),
            valores.len()
        );
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(&sql)?;
            let linhas = consulta.query_map(rusqlite::params_from_iter(valores.iter()), linha)?;
            let mut fluxos = Vec::new();
            for linha in linhas {
                fluxos.push(linha?);
            }
            Ok(fluxos)
        })
    }

    /// Altera nome, descrição, situação ou rascunho com `If-Match` (`esperada` = `version` lida).
    ///
    /// Versão diferente da gravada → [`DomainError::ConflitoDeVersao`] (412), sem gravar nada.
    /// Situação `archived` é recusada: fluxo só fica `active` ou `inactive`.
    pub fn atualizar(
        &self,
        org: OrgId,
        id: FluxoId,
        esperada: u64,
        mudanca: MudancaFluxo,
    ) -> Resultado<Fluxo> {
        if mudanca.status == Some(Status::Arquivado) {
            return Err(ErroStore::Dominio(DomainError::Validacao {
                campo: "status",
                motivo: "o fluxo fica `active` ou `inactive`".to_owned(),
            }));
        }
        if let Some(nome) = &mudanca.nome {
            validar_nome(nome)?;
        }
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let status = mudanca.status.map(Status::como_texto);
        let esperada_i64 = esperada as i64;
        let alteradas = self.0.escrita(move |conn| {
            conn.execute(
                "UPDATE fluxos
                    SET nome = COALESCE(?3, nome),
                        descricao = COALESCE(?4, descricao),
                        status = COALESCE(?5, status),
                        rascunho_fonte = COALESCE(?6, rascunho_fonte),
                        version = version + 1
                  WHERE org_id = ?1 AND id = ?2 AND version = ?7",
                params![
                    texto_org,
                    texto_id,
                    mudanca.nome,
                    mudanca.descricao,
                    status,
                    mudanca.rascunho_fonte,
                    esperada_i64
                ],
            )
            .map_err(|erro| ErroStore::da_escrita("nome", erro))
        })?;
        if alteradas == 0 {
            return Err(self.conflito(org, id, esperada));
        }
        self.obter(org, id)
    }

    /// Exclui o fluxo da org e devolve quantas execuções **ativas** ele tinha.
    ///
    /// - Sem execução ativa: exclui e devolve `0`.
    /// - Com execução ativa e `force = false`: recusa com [`DomainError::Conflito`] (409), com a
    ///   contagem na mensagem; nada muda.
    /// - Com `force = true`: as execuções ativas viram `abandonada` com motivo `fluxo_excluido`, cada
    ///   uma com o evento `execucao_abandonada` na Ficha, e o fluxo sai. Revisões e permissões saem
    ///   com ele (cascata); as **execuções e as Fichas ficam** (PRD §6.2), porque são o histórico do
    ///   atendimento e não têm chave estrangeira para o fluxo.
    ///
    /// Tudo numa escrita só: ou o fluxo sai com as execuções abandonadas, ou nada muda.
    pub fn excluir(&self, org: OrgId, id: FluxoId, force: bool) -> Resultado<u64> {
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        self.0.escrita(move |conn| {
            let existe: Option<i64> = conn
                .query_row(
                    "SELECT 1 FROM fluxos WHERE org_id = ?1 AND id = ?2",
                    params![texto_org, texto_id],
                    |linha| linha.get(0),
                )
                .optional()
                .map_err(ErroStore::from)?;
            if existe.is_none() {
                return Err(ErroStore::Dominio(DomainError::NaoEncontrado {
                    entidade: "fluxo",
                    id: texto_id.clone(),
                }));
            }
            let ativas = execucoes_ativas_do_fluxo(conn, &texto_org, &texto_id)?;
            if !ativas.is_empty() && !force {
                return Err(ErroStore::Dominio(DomainError::Conflito {
                    entidade: "fluxo",
                    id: texto_id.clone(),
                    motivo: format!(
                        "tem {} execução(ões) ativa(s) de atendimento; envie force=true para \
                         abandoná-las e excluir",
                        ativas.len()
                    ),
                }));
            }
            for (execucao, ficha) in &ativas {
                conn.execute(
                    "UPDATE fluxo_execucoes
                        SET status = 'abandonada', motivo_fim = ?3, finalizada_em = ?4,
                            version = version + 1
                      WHERE org_id = ?1 AND id = ?2",
                    params![texto_org, execucao, MOTIVO_EXCLUSAO_FLUXO, agora],
                )
                .map_err(ErroStore::from)?;
                registrar_evento_conn(
                    conn,
                    &texto_org,
                    ficha,
                    &NovoEventoFicha {
                        execucao_id: execucao.parse::<ExecucaoId>().ok(),
                        tipo: TipoEventoFicha::ExecucaoAbandonada,
                        ator_tipo: AtorEvento::Sistema,
                        ator_ref: None,
                        payload_json: format!("{{\"motivo\":\"{MOTIVO_EXCLUSAO_FLUXO}\"}}"),
                        chave_idempotencia: None,
                    },
                    agora,
                )?;
            }
            conn.execute(
                "DELETE FROM fluxos WHERE org_id = ?1 AND id = ?2",
                params![texto_org, texto_id],
            )
            .map_err(ErroStore::from)?;
            Ok(ativas.len() as u64)
        })
    }

    /// Libera o fluxo ao agente. Repetir a liberação do mesmo par não faz nada e não conta de novo.
    ///
    /// Recusa com `NaoEncontrado` se o fluxo ou o agente não é desta org, e com
    /// [`DomainError::TetoDeFluxosPorAgente`] se a liberação passaria de 20.
    pub fn liberar(&self, org: OrgId, agente: AgentId, fluxo: FluxoId) -> Resultado<()> {
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let texto_agente = agente.to_string();
        let texto_fluxo = fluxo.to_string();
        self.0.escrita(move |conn| {
            pertence(conn, "fluxos", "fluxo", &texto_org, &texto_fluxo)?;
            pertence(conn, "agents", "agente", &texto_org, &texto_agente)?;
            if permissao_existe(conn, &texto_org, &texto_agente, &texto_fluxo)? {
                return Ok(());
            }
            let atuais = contar_permissoes(conn, &texto_org, &texto_agente)?;
            if atuais >= TETO_FLUXOS_POR_AGENTE {
                return Err(erro_teto(agente, atuais.saturating_add(1)));
            }
            conn.execute(
                "INSERT INTO agente_fluxos (org_id, agent_id, fluxo_id, liberado_em)
                 VALUES (?1, ?2, ?3, ?4)",
                params![texto_org, texto_agente, texto_fluxo, agora],
            )
            .map_err(ErroStore::from)?;
            versao_do_agente_avanca(conn, &texto_org, &texto_agente, agora)
        })
    }

    /// Tira a permissão do agente. Revogar o que não estava liberado não faz nada.
    pub fn revogar(&self, org: OrgId, agente: AgentId, fluxo: FluxoId) -> Resultado<()> {
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let texto_agente = agente.to_string();
        let texto_fluxo = fluxo.to_string();
        self.0.escrita(move |conn| {
            let apagadas = conn
                .execute(
                    "DELETE FROM agente_fluxos
                      WHERE org_id = ?1 AND agent_id = ?2 AND fluxo_id = ?3",
                    params![texto_org, texto_agente, texto_fluxo],
                )
                .map_err(ErroStore::from)?;
            if apagadas > 0 {
                versao_do_agente_avanca(conn, &texto_org, &texto_agente, agora)?;
            }
            Ok(())
        })
    }

    /// Fluxos liberados ao agente, em ordem de slug.
    pub fn listar_do_agente(&self, org: OrgId, agente: AgentId) -> Resultado<Vec<Fluxo>> {
        let texto_org = org.to_string();
        let texto_agente = agente.to_string();
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(&format!(
                "SELECT {COLUNAS} FROM fluxos
                  WHERE org_id = ?1
                    AND id IN (SELECT fluxo_id FROM agente_fluxos
                                WHERE org_id = ?1 AND agent_id = ?2)
                  ORDER BY slug"
            ))?;
            let linhas = consulta.query_map(params![texto_org, texto_agente], linha)?;
            let mut fluxos = Vec::new();
            for linha in linhas {
                fluxos.push(linha?);
            }
            Ok(fluxos)
        })
    }

    /// Agentes que têm o fluxo liberado, em ordem de id. Fluxo de outra org → `NaoEncontrado`.
    pub fn listar_agentes_do_fluxo(&self, org: OrgId, fluxo: FluxoId) -> Resultado<Vec<AgentId>> {
        self.obter(org, fluxo)?;
        let texto_org = org.to_string();
        let texto_fluxo = fluxo.to_string();
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(
                "SELECT agent_id FROM agente_fluxos
                  WHERE org_id = ?1 AND fluxo_id = ?2
                  ORDER BY agent_id",
            )?;
            let linhas = consulta.query_map(params![texto_org, texto_fluxo], |linha| {
                id_da_coluna::<AgentId>(linha, 0)
            })?;
            let mut agentes = Vec::new();
            for linha in linhas {
                agentes.push(linha?);
            }
            Ok(agentes)
        })
    }

    /// Troca a lista inteira de fluxos do agente numa escrita só (PUT de permissão, PRD §6.2).
    ///
    /// `esperada` é a versão do **agente** lida antes (`If-Match`). Se não bater, `ConflitoDeVersao`
    /// e nada muda. A lista nova passa pelo teto, pela checagem de org de cada fluxo e pela troca:
    /// se qualquer passo falha, a lista antiga continua inteira (sem janela sem permissão). Fluxo
    /// repetido na lista conta uma vez.
    pub fn substituir_lista(
        &self,
        org: OrgId,
        agente: AgentId,
        esperada: u64,
        fluxos: &[FluxoId],
    ) -> Resultado<()> {
        let mut unicos: Vec<FluxoId> = Vec::new();
        for fluxo in fluxos {
            if !unicos.contains(fluxo) {
                unicos.push(*fluxo);
            }
        }
        if u32::try_from(unicos.len()).unwrap_or(u32::MAX) > TETO_FLUXOS_POR_AGENTE {
            return Err(erro_teto(
                agente,
                u32::try_from(unicos.len()).unwrap_or(u32::MAX),
            ));
        }
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let texto_agente = agente.to_string();
        let textos: Vec<String> = unicos.iter().map(ToString::to_string).collect();
        let esperada_i64 = esperada as i64;
        self.0.escrita(move |conn| {
            let avancou = conn
                .execute(
                    "UPDATE agents SET version = version + 1, updated_at = ?4
                      WHERE org_id = ?1 AND id = ?2 AND version = ?3",
                    params![texto_org, texto_agente, esperada_i64, agora],
                )
                .map_err(ErroStore::from)?;
            if avancou == 0 {
                return Err(conflito_ou_ausente(
                    conn,
                    "agents",
                    "agente",
                    &texto_agente,
                    &texto_org,
                    esperada,
                ));
            }
            for texto_fluxo in &textos {
                pertence(conn, "fluxos", "fluxo", &texto_org, texto_fluxo)?;
            }
            conn.execute(
                "DELETE FROM agente_fluxos WHERE org_id = ?1 AND agent_id = ?2",
                params![texto_org, texto_agente],
            )
            .map_err(ErroStore::from)?;
            for texto_fluxo in &textos {
                conn.execute(
                    "INSERT INTO agente_fluxos (org_id, agent_id, fluxo_id, liberado_em)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![texto_org, texto_agente, texto_fluxo, agora],
                )
                .map_err(ErroStore::from)?;
            }
            Ok(())
        })
    }

    /// Monta o erro certo quando o `UPDATE ... WHERE version = ?` não pegou nenhuma linha.
    fn conflito(&self, org: OrgId, id: FluxoId, esperada: u64) -> ErroStore {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        self.0
            .leitura(move |conn| {
                Ok(conflito_ou_ausente(
                    conn, "fluxos", "fluxo", &texto_id, &texto_org, esperada,
                ))
            })
            .unwrap_or_else(|erro| erro)
    }
}

/// Slug no formato da linguagem (`FLX13`): minúsculas, números e `_`, começa por letra, 2 a 64.
fn validar_slug(slug: &str) -> Resultado<()> {
    let tamanho = slug.chars().count();
    let mut caracteres = slug.chars();
    let valido = (2..=64).contains(&tamanho)
        && caracteres
            .next()
            .is_some_and(|primeiro| primeiro.is_ascii_lowercase())
        && caracteres.all(|caractere| {
            caractere.is_ascii_lowercase() || caractere.is_ascii_digit() || caractere == '_'
        });
    if valido {
        Ok(())
    } else {
        Err(ErroStore::Dominio(DomainError::Validacao {
            campo: "slug",
            motivo: "use minúsculas, números e `_`, começando por letra, com 2 a 64 caracteres"
                .to_owned(),
        }))
    }
}

fn validar_nome(nome: &str) -> Resultado<()> {
    if nome.trim().is_empty() {
        return Err(ErroStore::Dominio(DomainError::Validacao {
            campo: "nome",
            motivo: "não pode ficar vazio".to_owned(),
        }));
    }
    Ok(())
}

fn erro_teto(agente: AgentId, pedidos: u32) -> ErroStore {
    ErroStore::Dominio(DomainError::TetoDeFluxosPorAgente {
        agent_id: agente.to_string(),
        limite: TETO_FLUXOS_POR_AGENTE,
        pedidos,
    })
}

fn permissao_existe(conn: &Connection, org: &str, agente: &str, fluxo: &str) -> Resultado<bool> {
    let existe: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM agente_fluxos WHERE org_id = ?1 AND agent_id = ?2 AND fluxo_id = ?3",
            params![org, agente, fluxo],
            |linha| linha.get(0),
        )
        .optional()
        .map_err(ErroStore::from)?;
    Ok(existe.is_some())
}

fn contar_permissoes(conn: &Connection, org: &str, agente: &str) -> Resultado<u32> {
    let total: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM agente_fluxos WHERE org_id = ?1 AND agent_id = ?2",
            params![org, agente],
            |linha| linha.get(0),
        )
        .map_err(ErroStore::from)?;
    Ok(u32::try_from(total).unwrap_or(u32::MAX))
}

/// Ids de (execução, ficha) das execuções `ativa` do fluxo, na org.
fn execucoes_ativas_do_fluxo(
    conn: &Connection,
    org: &str,
    fluxo: &str,
) -> Resultado<Vec<(String, String)>> {
    let mut consulta = conn
        .prepare(
            "SELECT id, ficha_id FROM fluxo_execucoes
              WHERE org_id = ?1 AND fluxo_id = ?2 AND status = 'ativa'
              ORDER BY iniciada_em",
        )
        .map_err(ErroStore::from)?;
    let linhas = consulta
        .query_map(params![org, fluxo], |linha| {
            Ok((linha.get::<_, String>(0)?, linha.get::<_, String>(1)?))
        })
        .map_err(ErroStore::from)?;
    let mut ativas = Vec::new();
    for linha in linhas {
        ativas.push(linha.map_err(ErroStore::from)?);
    }
    Ok(ativas)
}

/// A permissão mudou: a versão do agente avança, para o `If-Match` de quem leu antes dar `412`.
fn versao_do_agente_avanca(
    conn: &Connection,
    org: &str,
    agente: &str,
    agora: i64,
) -> Resultado<()> {
    conn.execute(
        "UPDATE agents SET version = version + 1, updated_at = ?3
          WHERE org_id = ?1 AND id = ?2",
        params![org, agente, agora],
    )
    .map_err(ErroStore::from)?;
    Ok(())
}

/// Lê um fluxo da linha do resultado.
fn linha(linha: &Row<'_>) -> rusqlite::Result<Fluxo> {
    Ok(Fluxo {
        id: id_da_coluna(linha, 0)?,
        org_id: id_da_coluna(linha, 1)?,
        slug: linha.get(2)?,
        nome: linha.get(3)?,
        descricao: linha.get(4)?,
        status: status_da_coluna(linha, 5)?,
        rascunho_fonte: linha.get(6)?,
        revisao_atual: linha
            .get::<_, Option<i64>>(7)?
            .map(|numero| numero.max(0) as u64),
        versao: u64_da_coluna(linha, 8)?,
    })
}
