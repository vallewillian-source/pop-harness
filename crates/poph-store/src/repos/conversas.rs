//! Repositório de conversas (PRD §7.2 e D12).
//!
//! `status = translated` (transferida) significa: mensagem do cliente é gravada, o agente não
//! responde até alguém reabrir pela CLI/Console.

use poph_types::{AgentId, ConversationId, DepartmentId, OrgId};
use rusqlite::{OptionalExtension, Row, params};

use crate::Store;
use crate::erro::{ErroStore, Resultado};
use crate::modelo::{Conversa, NovaConversa, Pagina, StatusConversa};
use crate::repos::{conflito_ou_ausente, id_da_coluna, id_opcional, pertence, u64_da_coluna};

const COLUNAS: &str = "id, org_id, agent_id, integration_id, channel, external_peer, status,
    target_department_id, summary, version, created_at, updated_at, last_message_at, client_name";

/// Tamanho máximo do nome do cliente gravado, em caracteres (MX-32).
pub const LIMITE_NOME_CLIENTE: usize = 100;

/// Motivo gravado na Ficha quando o administrador encerra a conversa (`close`, MX-06 + FX-07).
pub const MOTIVO_FECHAMENTO_ADMIN: &str = "encerrada pelo administrador";

/// Motivo gravado na Ficha quando o caminho genérico de mudança de situação põe a conversa em
/// `closed` (`mudar_status`), que não recebe autor.
pub const MOTIVO_FECHAMENTO_SITUACAO: &str = "conversa fechada por mudança de situação";

/// Limpa o nome do cliente que veio do WhatsApp (`pushName`) antes de gravar (MX-32).
///
/// Regras: qualquer espaço em branco (inclusive quebra de linha e tabulação) vira espaço; espaços
/// repetidos viram um só; caractere de controle e de largura zero ou de direção de texto some
/// (U+200B–U+200F, U+202A–U+202E, U+2066–U+2069); sem espaço nas pontas; no máximo
/// [`LIMITE_NOME_CLIENTE`] caracteres. Devolve `None` quando nada sobra (nome vazio não é gravado).
pub fn normalizar_nome_cliente(bruto: &str) -> Option<String> {
    let mut limpo = String::with_capacity(bruto.len());
    let mut ultimo_foi_espaco = true;
    for caractere in bruto.chars() {
        if caractere.is_whitespace() {
            if !ultimo_foi_espaco {
                limpo.push(' ');
                ultimo_foi_espaco = true;
            }
            continue;
        }
        if caractere.is_control() || eh_marca_invisivel(caractere) {
            continue;
        }
        limpo.push(caractere);
        ultimo_foi_espaco = false;
    }
    let cortado: String = limpo
        .trim_end()
        .chars()
        .take(LIMITE_NOME_CLIENTE)
        .collect::<String>();
    let cortado = cortado.trim_end().to_owned();
    (!cortado.is_empty()).then_some(cortado)
}

fn eh_marca_invisivel(caractere: char) -> bool {
    matches!(
        caractere,
        '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

/// Grava o nome do cliente dentro de uma conexão que já está em transação (MX-32).
///
/// É o miolo de [`Conversas::definir_nome_cliente`] e também o que `receber_do_canal` chama, dentro
/// da própria transação da mensagem. Não abre transação. Nome que normaliza para vazio devolve
/// `false` sem escrever; o `UPDATE` só age quando o valor muda, sobe `updated_at` e não mexe em
/// `version` nem em `last_message_at`.
pub(crate) fn gravar_nome_cliente(
    conn: &rusqlite::Connection,
    texto_org: &str,
    texto_id: &str,
    nome: &str,
    agora: i64,
) -> Resultado<bool> {
    let Some(nome) = normalizar_nome_cliente(nome) else {
        return Ok(false);
    };
    let alteradas = conn
        .execute(
            "UPDATE conversations
                SET client_name = ?3, updated_at = ?4
              WHERE org_id = ?1 AND id = ?2 AND client_name IS NOT ?3",
            params![texto_org, texto_id, nome, agora],
        )
        .map_err(ErroStore::from)?;
    Ok(alteradas > 0)
}

/// Filtros da listagem administrativa de conversas (RP-02). Campo vazio não filtra.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FiltroConversas {
    /// Situação exata: `open`, `transferred` ou `closed`.
    pub status: Option<StatusConversa>,
    /// Identificador do cliente no canal (`external_peer`), exato.
    pub external_peer: Option<String>,
}

/// Ordem da listagem administrativa de conversas (RP-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OrdemConversas {
    /// Por id crescente, paginada por cursor: a ordem de sempre.
    #[default]
    Id,
    /// Atividade mais recente primeiro, `COALESCE(last_message_at, created_at)` decrescente, com
    /// desempate por id decrescente. Não usa cursor.
    Recente,
}

impl Conversa {
    /// Identificador do cliente que a sala do Matrix usa como `client_identifier` no metadata:
    /// o `external_peer` sem espaço nas pontas, ou o id da conversa quando ele está vazio.
    ///
    /// A listagem filtra por `external_peer` com este mesmo valor para achar o histórico do
    /// cliente (RP-02). Os módulos Matrix ainda calculam a regra em cada chamador; o teste
    /// `dados_element` fixa a igualdade entre as duas pontas.
    pub fn identificador_cliente(&self) -> String {
        self.peer_externo
            .as_deref()
            .map(str::trim)
            .filter(|peer| !peer.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| self.id.to_string())
    }
}

/// Valor de parâmetro da consulta da listagem.
type ValorSql = rusqlite::types::Value;

/// Monta a consulta da listagem administrativa. Só entra no `WHERE` o que foi pedido, para o
/// SQLite usar `ix_conversations_org_peer_status` quando há `external_peer`.
fn montar_listagem(
    org: String,
    filtro: &FiltroConversas,
    ordem: OrdemConversas,
    apos: Option<String>,
    limite: i64,
) -> (String, Vec<ValorSql>) {
    let mut condicoes = vec!["org_id = ?1".to_owned()];
    let mut valores = vec![ValorSql::Text(org)];
    if let Some(status) = filtro.status {
        valores.push(ValorSql::Text(status.como_texto().to_owned()));
        condicoes.push(format!("status = ?{}", valores.len()));
    }
    if let Some(peer) = &filtro.external_peer {
        valores.push(ValorSql::Text(peer.clone()));
        condicoes.push(format!("external_peer = ?{}", valores.len()));
    }
    let ordenacao = match ordem {
        OrdemConversas::Id => {
            if let Some(apos) = apos {
                valores.push(ValorSql::Text(apos));
                condicoes.push(format!("id > ?{}", valores.len()));
            }
            "id"
        }
        OrdemConversas::Recente => "COALESCE(last_message_at, created_at) DESC, id DESC",
    };
    valores.push(ValorSql::Integer(limite));
    // Com cliente, a origem fixa o índice `ix_conversations_org_peer_id`. Sem ele, o planejador
    // escolhe o índice de situação para a ordenação por atividade e lê as conversas abertas da org
    // inteira, filtrando o cliente linha a linha. Sem cliente, a escolha do planejador fica como está.
    let origem = if filtro.external_peer.is_some() {
        "conversations INDEXED BY ix_conversations_org_peer_id"
    } else {
        "conversations"
    };
    let sql = format!(
        "SELECT {COLUNAS} FROM {origem} WHERE {} ORDER BY {ordenacao} LIMIT ?{}",
        condicoes.join(" AND "),
        valores.len()
    );
    (sql, valores)
}

/// Acesso às conversas.
pub struct Conversas<'a>(pub(crate) &'a Store);

impl Conversas<'_> {
    /// Reabre uma conversa e grava a auditoria no mesmo comando de escrita.
    ///
    /// **Conversa `closed` não volta** (PRD L13, FX-07): devolve
    /// [`poph_types::DomainError::ConversaEncerrada`] (409 `conversa_encerrada`). O caminho para o
    /// cliente voltar a ser atendido é a mensagem nova dele, que abre outra conversa **e outra
    /// Ficha**. Conversa `transferred` continua reabrindo normalmente.
    ///
    /// Ordem das recusas, para o erro não mentir sobre a causa: conversa que não existe na org →
    /// `NaoEncontrado` (404); `If-Match` velho → `ConflitoDeVersao` (412); conversa encerrada →
    /// `ConversaEncerrada` (409). Um pedido recusado não muda a conversa nem grava auditoria.
    pub fn reabrir_auditado(
        &self,
        org: OrgId,
        id: ConversationId,
        esperada: u64,
        ator: String,
    ) -> Resultado<Conversa> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let agora = self.0.agora();
        let audit_id = poph_types::AuditId::generate().to_string();
        let esperada_i64 = i64::try_from(esperada).unwrap_or(i64::MAX);
        let fechada = StatusConversa::Fechada.como_texto();
        self.0.escrita(move |conn| {
            let atual: Option<(i64, String)> = conn
                .query_row(
                    "SELECT version, status FROM conversations WHERE org_id = ?1 AND id = ?2",
                    params![texto_org, texto_id],
                    |linha| Ok((linha.get(0)?, linha.get(1)?)),
                )
                .optional()
                .map_err(ErroStore::from)?;
            let Some((versao_atual, situacao)) = atual else {
                return super::exigir::<()>(None, "conversa", &texto_id);
            };
            if versao_atual != esperada_i64 {
                return Err(ErroStore::Dominio(
                    poph_types::DomainError::ConflitoDeVersao {
                        entidade: "conversa",
                        id: texto_id.clone(),
                        esperada,
                        atual: u64::try_from(versao_atual).unwrap_or(0),
                    },
                ));
            }
            if situacao == fechada {
                return Err(ErroStore::Dominio(poph_types::DomainError::ConversaEncerrada {
                    id: texto_id.clone(),
                }));
            }
            let alteradas = conn.execute("UPDATE conversations SET status='open', version=version+1, updated_at=?3 WHERE org_id=?1 AND id=?2 AND version=?4", params![texto_org, texto_id, agora, esperada_i64])?;
            if alteradas == 0 {
                return Err(conflito_ou_ausente(conn, "conversations", "conversa", &texto_id, &texto_org, esperada));
            }
            conn.execute("INSERT INTO audit_log (id, org_id, actor_kind, actor_id, action, entity, entity_id, detail_json, created_at) VALUES (?1, ?2, 'admin_token', ?3, 'reopen', 'conversation', ?4, '{}', ?5)", params![audit_id, texto_org, ator, texto_id, agora])?;
            Ok(())
        })?;
        self.obter(org, id)
    }

    /// Fecha uma conversa e grava a auditoria (`close`) na mesma transação de escrita (MX-06).
    ///
    /// Ordem das recusas, todas antes de qualquer gravação: conversa que não existe na org →
    /// `NaoEncontrado` (404); `If-Match` diferente da versão atual → `ConflitoDeVersao` (412);
    /// conversa já `closed` → `Conflito` (409). Um pedido recusado não muda a conversa nem grava
    /// auditoria. Quem fecha é o administrador (`admin_token`), como na reabertura.
    ///
    /// Na mesma escrita, **fecha a Ficha** do atendimento (PRD L5, FX-07): a Ficha fica `closed` com
    /// `fechada_por = admin`, a execução de fluxo ativa é abandonada com o motivo
    /// `atendimento_encerrado` e os eventos entram na linha do tempo. Se qualquer parte falhar, o
    /// savepoint do escritor volta inteiro: não fica conversa fechada com Ficha aberta.
    pub fn fechar_auditado(
        &self,
        org: OrgId,
        id: ConversationId,
        esperada: u64,
        ator: String,
    ) -> Resultado<Conversa> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let agora = self.0.agora();
        let audit_id = poph_types::AuditId::generate().to_string();
        let esperada_i64 = i64::try_from(esperada).unwrap_or(i64::MAX);
        let fechada = StatusConversa::Fechada.como_texto();
        // FX-40: os contadores das execuções atravessam o comando de escrita (os handles clonam).
        let metricas = self.0.metricas_de_execucao();
        self.0.escrita(move |conn| {
            let atual: Option<(i64, String)> = conn
                .query_row(
                    "SELECT version, status FROM conversations WHERE org_id = ?1 AND id = ?2",
                    params![texto_org, texto_id],
                    |linha| Ok((linha.get(0)?, linha.get(1)?)),
                )
                .optional()
                .map_err(ErroStore::from)?;
            let Some((versao_atual, situacao)) = atual else {
                return super::exigir::<()>(None, "conversa", &texto_id);
            };
            if versao_atual != esperada_i64 {
                return Err(ErroStore::Dominio(
                    poph_types::DomainError::ConflitoDeVersao {
                        entidade: "conversa",
                        id: texto_id.clone(),
                        esperada,
                        atual: u64::try_from(versao_atual).unwrap_or(0),
                    },
                ));
            }
            if situacao == fechada {
                return Err(ErroStore::Dominio(poph_types::DomainError::Conflito {
                    entidade: "conversa",
                    id: texto_id.clone(),
                    motivo: "a conversa já está fechada".to_owned(),
                }));
            }
            let alteradas = conn
                .execute(
                    "UPDATE conversations
                        SET status = 'closed', version = version + 1, updated_at = ?3
                      WHERE org_id = ?1 AND id = ?2 AND version = ?4",
                    params![texto_org, texto_id, agora, esperada_i64],
                )
                .map_err(ErroStore::from)?;
            if alteradas == 0 {
                return Err(conflito_ou_ausente(
                    conn,
                    "conversations",
                    "conversa",
                    &texto_id,
                    &texto_org,
                    esperada,
                ));
            }
            conn.execute(
                "INSERT INTO audit_log
                    (id, org_id, actor_kind, actor_id, action, entity, entity_id, detail_json,
                     created_at)
                 VALUES (?1, ?2, 'admin_token', ?3, 'close', 'conversation', ?4, '{}', ?5)",
                params![audit_id, texto_org, ator, texto_id, agora],
            )
            .map_err(ErroStore::from)?;
            // A Ficha do atendimento fecha junto, no mesmo savepoint (PRD L5).
            super::fichas::fechar_ficha_da_conversa(
                conn,
                &texto_org,
                &texto_id,
                poph_types::FechadaPor::Admin,
                MOTIVO_FECHAMENTO_ADMIN,
                agora,
                metricas.as_ref(),
            )?;
            Ok(())
        })?;
        self.obter(org, id)
    }

    /// Encerra pelo agente e audita junto com a Ficha e o abandono da execução.
    /// Devolve `false` quando a conversa já estava encerrada.
    pub fn fechar_pelo_agente(
        &self,
        org: OrgId,
        id: ConversationId,
        agente: AgentId,
        motivo: String,
    ) -> Resultado<bool> {
        if motivo.trim().is_empty() {
            return Err(ErroStore::Dominio(poph_types::DomainError::Validacao {
                campo: "motivo",
                motivo: "informe o motivo do encerramento".to_owned(),
            }));
        }
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let texto_agente = agente.to_string();
        let agora = self.0.agora();
        let audit_id = poph_types::AuditId::generate().to_string();
        let metricas = self.0.metricas_de_execucao();
        self.0.escrita(move |conn| {
            let situacao: Option<String> = conn.query_row(
                "SELECT status FROM conversations WHERE org_id=?1 AND id=?2 AND agent_id=?3",
                params![texto_org, texto_id, texto_agente], |linha| linha.get(0),
            ).optional()?;
            let Some(situacao) = situacao else {
                return super::exigir::<bool>(None, "conversa", &texto_id);
            };
            if situacao == "closed" { return Ok(false); }
            conn.execute(
                "UPDATE conversations SET status='closed', version=version+1, updated_at=?3 WHERE org_id=?1 AND id=?2",
                params![texto_org, texto_id, agora],
            )?;
            super::fichas::fechar_ficha_da_conversa(
                conn, &texto_org, &texto_id, poph_types::FechadaPor::Agente, &motivo, agora,
                metricas.as_ref(),
            )?;
            let detalhe = serde_json::json!({"motivo": motivo, "ator": "agente"}).to_string();
            conn.execute(
                "INSERT INTO audit_log (id, org_id, actor_kind, actor_id, action, entity, entity_id, detail_json, created_at) VALUES (?1, ?2, 'system', ?3, 'close', 'conversation', ?4, ?5, ?6)",
                params![audit_id, texto_org, texto_agente, texto_id, detalhe, agora],
            )?;
            Ok(true)
        })
    }

    /// Marca a conversa como **transferida** para humano e grava a auditoria na mesma transação
    /// (HR-29, D12).
    ///
    /// Diferente de [`Self::mudar_status`], não exige `If-Match`: a transferência é consequência do
    /// turno (o Jev decidiu), não edição administrativa, e não pode falhar porque a tela de alguém
    /// estava aberta. A versão sobe do mesmo jeito, então um `PATCH` com versão antiga continua
    /// recebendo 412. O departamento alvo só é trocado quando informado.
    pub fn transferir(
        &self,
        org: OrgId,
        id: ConversationId,
        departamento_alvo: Option<DepartmentId>,
        ator_id: Option<String>,
        detalhe_json: String,
    ) -> Resultado<Conversa> {
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let audit_id = poph_types::AuditId::generate().to_string();
        self.0.escrita(move |conn| {
            if let Some(departamento) = departamento_alvo {
                pertence(
                    conn,
                    "departments",
                    "departamento",
                    &texto_org,
                    &departamento.to_string(),
                )?;
            }
            let alteradas = conn.execute(
                "UPDATE conversations
                    SET status = 'transferred',
                        target_department_id = COALESCE(?3, target_department_id),
                        version = version + 1,
                        updated_at = ?4
                  WHERE org_id = ?1 AND id = ?2",
                params![
                    texto_org,
                    texto_id,
                    departamento_alvo.map(|dep| dep.to_string()),
                    agora
                ],
            )?;
            if alteradas == 0 {
                return super::exigir::<()>(None, "conversa", &texto_id);
            }
            conn.execute(
                "INSERT INTO audit_log
                    (id, org_id, actor_kind, actor_id, action, entity, entity_id, detail_json,
                     created_at)
                 VALUES (?1, ?2, 'system', ?3, 'transfer', 'conversation', ?4, ?5, ?6)",
                params![audit_id, texto_org, ator_id, texto_id, detalhe_json, agora],
            )
            .map_err(ErroStore::from)?;
            Ok(())
        })?;
        self.obter(org, id)
    }

    /// Abre uma conversa.
    /// Abre uma conversa **e a Ficha do atendimento**, na mesma escrita (PRD L4, FX-07).
    ///
    /// A Ficha é criada aqui, e não em cada chamador, porque este é o caminho único de criação do
    /// repositório (o outro é a recepção do canal, em [`super::CaixaEntrada::receber_do_canal`], que
    /// também cria a Ficha na escrita dela). Conversa sem Ficha deixaria de cumprir "todo atendimento
    /// tem registro" e a linha do tempo do atendimento começaria sem o evento de abertura.
    pub fn criar(&self, org: OrgId, nova: NovaConversa) -> Resultado<Conversa> {
        let id = ConversationId::generate();
        let agora = self.0.agora();
        let texto_id = id.to_string();
        let texto_org = org.to_string();
        let texto_agente = nova.agent_id.to_string();
        let canal = nova.canal;
        let peer = nova.peer_externo;
        self.0.escrita(move |conn| {
            if let Some(departamento) = nova.departamento_alvo {
                pertence(
                    conn,
                    "departments",
                    "departamento",
                    &texto_org,
                    &departamento.to_string(),
                )?;
            }
            conn.execute(
                "INSERT INTO conversations
                    (id, org_id, agent_id, integration_id, channel, external_peer, status,
                     target_department_id, summary, version, created_at, updated_at, last_message_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'open', ?7, NULL, 1, ?8, ?8, NULL)",
                params![
                    texto_id,
                    texto_org,
                    &texto_agente,
                    nova.integration_id.map(|id| id.to_string()),
                    &canal,
                    peer.as_deref(),
                    nova.departamento_alvo.map(|dep| dep.to_string()),
                    agora
                ],
            )
            .map_err(ErroStore::from)?;
            super::fichas::criar_ficha_da_conversa(
                conn,
                &texto_org,
                &texto_id,
                &texto_agente,
                &canal,
                peer.as_deref(),
                agora,
            )?;
            Ok(())
        })?;
        self.obter(org, id)
    }

    /// Leitura incremental do espelhamento (PRD §7 N3): conversas em ordem crescente de
    /// `(updated_at, id)`, depois do cursor `apos`, no máximo `limite` (teto de 1000).
    ///
    /// O par evita perder conversa que tenha o mesmo `updated_at` (milissegundo) da última já
    /// conferida: o id desempata. `None` começa do início. Usa o leitor, não o escritor.
    /// O cursor guardado em `matrix_estado` é o último par devolvido.
    pub fn alteradas_desde(
        &self,
        apos: Option<(i64, ConversationId)>,
        limite: u32,
    ) -> Resultado<Vec<Conversa>> {
        let (instante, id) = match apos {
            Some((instante, id)) => (instante, id.to_string()),
            None => (i64::MIN, String::new()),
        };
        let limite = i64::from(limite.clamp(1, 1000));
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(&format!(
                "SELECT {COLUNAS} FROM conversations
                  WHERE (updated_at, id) > (?1, ?2)
                  ORDER BY updated_at, id
                  LIMIT ?3"
            ))?;
            let linhas = consulta.query_map(params![instante, id, limite], linha)?;
            let mut conversas = Vec::new();
            for linha in linhas {
                conversas.push(linha?);
            }
            Ok(conversas)
        })
    }

    /// Busca pelo id dentro da org.
    pub fn obter(&self, org: OrgId, id: ConversationId) -> Resultado<Conversa> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let encontrada = self.0.leitura(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUNAS} FROM conversations WHERE id = ?1 AND org_id = ?2"),
                params![texto_id, texto_org],
                linha,
            )
            .optional()
            .map_err(ErroStore::from)
        })?;
        super::exigir(encontrada, "conversa", &id.to_string())
    }

    /// Lista as conversas da org, opcionalmente filtrando por situação.
    pub fn listar(
        &self,
        org: OrgId,
        status: Option<StatusConversa>,
        pagina: Pagina,
    ) -> Resultado<Vec<Conversa>> {
        let texto_org = org.to_string();
        let status = status.map(StatusConversa::como_texto);
        let apos = pagina.apos.map(|id| id.to_string());
        let limite = i64::from(pagina.limite);
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(&format!(
                "SELECT {COLUNAS} FROM conversations
                  WHERE org_id = ?1
                    AND (?2 IS NULL OR status = ?2)
                    AND (?3 IS NULL OR id > ?3)
                  ORDER BY id LIMIT ?4"
            ))?;
            let linhas = consulta.query_map(params![texto_org, status, apos, limite], linha)?;
            let mut conversas = Vec::new();
            for linha in linhas {
                conversas.push(linha?);
            }
            Ok(conversas)
        })
    }

    /// Lista a org com filtros opcionais (RP-02), pela ordem de id e paginada por cursor. Sem filtro
    /// devolve o mesmo que [`Self::listar`] sem situação.
    pub fn listar_filtrado(
        &self,
        org: OrgId,
        filtro: &FiltroConversas,
        pagina: Pagina,
    ) -> Resultado<Vec<Conversa>> {
        let apos = pagina.apos.map(|id| id.to_string());
        let (sql, valores) = montar_listagem(
            org.to_string(),
            filtro,
            OrdemConversas::Id,
            apos,
            i64::from(pagina.limite),
        );
        self.consultar(sql, valores)
    }

    /// Lista a org com filtros opcionais, da atividade mais recente para a mais antiga, sem
    /// cursor (RP-02, `sort=recent`). `limite` fica entre 1 e 200.
    pub fn listar_recentes(
        &self,
        org: OrgId,
        filtro: &FiltroConversas,
        limite: u32,
    ) -> Resultado<Vec<Conversa>> {
        let (sql, valores) = montar_listagem(
            org.to_string(),
            filtro,
            OrdemConversas::Recente,
            None,
            i64::from(limite.clamp(1, 200)),
        );
        self.consultar(sql, valores)
    }

    /// Plano que o SQLite escolhe para a listagem (`EXPLAIN QUERY PLAN`), uma linha por passo.
    /// Diagnóstico: o teste do índice usa esta função para provar que o filtro por cliente acha as
    /// conversas pelo índice e não varre a tabela inteira.
    pub fn plano_da_listagem(
        &self,
        org: OrgId,
        filtro: &FiltroConversas,
        ordem: OrdemConversas,
    ) -> Resultado<Vec<String>> {
        let (sql, valores) = montar_listagem(org.to_string(), filtro, ordem, None, 50);
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
            let linhas = consulta
                .query_map(rusqlite::params_from_iter(valores.iter()), |linha| {
                    linha.get::<_, String>(3)
                })?;
            let mut passos = Vec::new();
            for linha in linhas {
                passos.push(linha?);
            }
            Ok(passos)
        })
    }

    fn consultar(&self, sql: String, valores: Vec<ValorSql>) -> Resultado<Vec<Conversa>> {
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(&sql)?;
            let linhas = consulta.query_map(rusqlite::params_from_iter(valores.iter()), linha)?;
            let mut conversas = Vec::new();
            for linha in linhas {
                conversas.push(linha?);
            }
            Ok(conversas)
        })
    }

    /// Conversas de um agente num canal, **mais recentes primeiro** (HR-22, chat de teste).
    ///
    /// Leitura só de metadados: as mensagens e os turnos ficam para quem abrir a conversa.
    pub fn listar_por_agente_canal(
        &self,
        org: OrgId,
        agente: AgentId,
        canal: &str,
        apos: Option<ConversationId>,
        limite: u32,
    ) -> Resultado<Vec<Conversa>> {
        let texto_org = org.to_string();
        let texto_agente = agente.to_string();
        let canal = canal.to_owned();
        let apos = apos.map(|id| id.to_string());
        let limite = i64::from(limite.clamp(1, 200));
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(&format!(
                "SELECT {COLUNAS} FROM conversations
                  WHERE org_id = ?1 AND agent_id = ?2 AND channel = ?3
                    AND (?4 IS NULL OR id < ?4)
                  ORDER BY id DESC LIMIT ?5"
            ))?;
            let linhas =
                consulta.query_map(params![texto_org, texto_agente, canal, apos, limite], linha)?;
            let mut conversas = Vec::new();
            for linha in linhas {
                conversas.push(linha?);
            }
            Ok(conversas)
        })
    }

    /// Muda situação/departamento alvo com `If-Match` (reabrir, transferir, fechar).
    ///
    /// Duas mudanças da FX-07, para o ciclo da Ficha não quebrar:
    ///
    /// - **fechar** (`StatusConversa::Fechada`) fecha a Ficha do atendimento na mesma escrita, como
    ///   [`Self::fechar_auditado`], com `fechada_por = sistema` (este caminho genérico não recebe
    ///   autor) e a execução ativa abandonada;
    /// - **reabrir** (`StatusConversa::Aberta`) a partir de `closed` é recusado com
    ///   [`poph_types::DomainError::ConversaEncerrada`], como em [`Self::reabrir_auditado`] (PRD L13):
    ///   senão a conversa voltaria `open` com a Ficha `closed`, dois estados que não coexistem.
    pub fn mudar_status(
        &self,
        org: OrgId,
        id: ConversationId,
        esperada: u64,
        status: StatusConversa,
        departamento_alvo: Option<DepartmentId>,
    ) -> Resultado<Conversa> {
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let esperada_i64 = esperada as i64;
        let fechada = StatusConversa::Fechada.como_texto();
        // FX-40: fechar por aqui também abandona a execução ativa da Ficha (conta no store).
        let metricas = self.0.metricas_de_execucao();
        self.0.escrita(move |conn| {
            if let Some(departamento) = departamento_alvo {
                pertence(
                    conn,
                    "departments",
                    "departamento",
                    &texto_org,
                    &departamento.to_string(),
                )?;
            }
            let situacao_atual: Option<String> = conn
                .query_row(
                    "SELECT status FROM conversations WHERE org_id = ?1 AND id = ?2",
                    params![texto_org, texto_id],
                    |linha| linha.get(0),
                )
                .optional()
                .map_err(ErroStore::from)?;
            if status == StatusConversa::Aberta && situacao_atual.as_deref() == Some(fechada) {
                return Err(ErroStore::Dominio(
                    poph_types::DomainError::ConversaEncerrada {
                        id: texto_id.clone(),
                    },
                ));
            }
            let alteradas = conn
                .execute(
                    "UPDATE conversations
                        SET status = ?3,
                            target_department_id = COALESCE(?4, target_department_id),
                            version = version + 1,
                            updated_at = ?5
                      WHERE org_id = ?1 AND id = ?2 AND version = ?6",
                    params![
                        texto_org,
                        texto_id,
                        status.como_texto(),
                        departamento_alvo.map(|dep| dep.to_string()),
                        agora,
                        esperada_i64
                    ],
                )
                .map_err(ErroStore::from)?;
            if alteradas == 0 {
                return Err(conflito_ou_ausente(
                    conn,
                    "conversations",
                    "conversa",
                    &texto_id,
                    &texto_org,
                    esperada,
                ));
            }
            if status == StatusConversa::Fechada {
                super::fichas::fechar_ficha_da_conversa(
                    conn,
                    &texto_org,
                    &texto_id,
                    poph_types::FechadaPor::Sistema,
                    MOTIVO_FECHAMENTO_SITUACAO,
                    agora,
                    metricas.as_ref(),
                )?;
            }
            Ok(())
        })?;
        self.obter(org, id)
    }

    /// Marca movimento na conversa **sem** mudar a versão: é escrituração do turno, não edição
    /// administrativa, e não pode invalidar o `If-Match` de quem está com a tela aberta.
    pub fn tocar(&self, org: OrgId, id: ConversationId, quando: i64) -> Resultado<()> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        self.0.escrita(move |conn| {
            conn.execute(
                "UPDATE conversations
                    SET last_message_at = ?3, updated_at = ?3
                  WHERE org_id = ?1 AND id = ?2",
                params![texto_org, texto_id, quando],
            )
            .map(|_| ())
            .map_err(ErroStore::from)
        })
    }

    /// Grava o resumo rolante (HR-14). Também não muda a versão (é derivado das mensagens).
    pub fn definir_resumo(&self, org: OrgId, id: ConversationId, resumo: &str) -> Resultado<()> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let resumo = resumo.to_owned();
        self.0.escrita(move |conn| {
            conn.execute(
                "UPDATE conversations SET summary = ?3 WHERE org_id = ?1 AND id = ?2",
                params![texto_org, texto_id, resumo],
            )
            .map(|_| ())
            .map_err(ErroStore::from)
        })
    }

    /// Grava o nome do cliente vindo do WhatsApp (MX-32). Devolve `true` quando o valor mudou.
    ///
    /// O nome passa por [`normalizar_nome_cliente`]; resultado vazio devolve `false` sem escrever.
    /// O `UPDATE` só age quando o valor é diferente do gravado: sobe `updated_at` (é o que faz o
    /// cursor de conversas do espelho enxergar a mudança), mas **não** sobe `version` (não gera
    /// conflito de `If-Match` no Console) e não mexe em `last_message_at`. Sempre filtrado por org.
    pub fn definir_nome_cliente(
        &self,
        org: OrgId,
        id: ConversationId,
        nome: &str,
    ) -> Resultado<bool> {
        let nome = nome.to_owned();
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        self.0
            .escrita(move |conn| gravar_nome_cliente(conn, &texto_org, &texto_id, &nome, agora))
    }

    /// Agente dono da conversa (usado no turno para saber qual config carregar).
    pub fn agente(&self, org: OrgId, id: ConversationId) -> Resultado<AgentId> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let agente = self.0.leitura(move |conn| {
            conn.query_row(
                "SELECT agent_id FROM conversations WHERE org_id = ?1 AND id = ?2",
                params![texto_org, texto_id],
                |linha| id_da_coluna::<AgentId>(linha, 0),
            )
            .optional()
            .map_err(ErroStore::from)
        })?;
        super::exigir(agente, "conversa", &id.to_string())
    }
}

/// Integração de origem, quando houver.
fn linha(linha: &Row<'_>) -> rusqlite::Result<Conversa> {
    Ok(Conversa {
        id: id_da_coluna(linha, 0)?,
        org_id: id_da_coluna(linha, 1)?,
        agent_id: id_da_coluna(linha, 2)?,
        integration_id: id_opcional(linha, 3)?,
        canal: linha.get(4)?,
        peer_externo: linha.get(5)?,
        status: StatusConversa::de_texto(&linha.get::<_, String>(6)?).map_err(|erro| {
            rusqlite::Error::FromSqlConversionFailure(
                6,
                rusqlite::types::Type::Text,
                Box::new(erro),
            )
        })?,
        departamento_alvo: id_opcional(linha, 7)?,
        resumo: linha.get(8)?,
        versao: u64_da_coluna(linha, 9)?,
        criado_em: linha.get(10)?,
        atualizado_em: linha.get(11)?,
        ultima_mensagem_em: linha.get(12)?,
        nome_cliente: linha.get(13)?,
    })
}
