//! Repositório de Fichas e da linha do tempo do atendimento (FX-07, PRD §6.1 e §7.2).
//!
//! A invariante do módulo vive aqui: **toda conversa tem exatamente uma Ficha**, criada na mesma
//! escrita que cria a conversa, e **fechar a conversa fecha a Ficha** na mesma transação, junto com
//! o abandono da execução de fluxo ativa (PRD L4, L5 e L6). Por isso este arquivo tem, além da API
//! pública de leitura e escrita, as funções internas que os outros repositórios chamam **dentro da
//! própria escrita deles**:
//!
//! - [`criar_ficha_da_conversa`] — usada por [`super::Conversas::criar`] e pela recepção do canal
//!   ([`super::CaixaEntrada::receber_do_canal`], o caminho que o WhatsApp usa de fato);
//! - [`fechar_ficha_da_conversa`] — usada por todo caminho que põe a conversa em `closed`;
//! - [`registrar_evento_conn`] — a escrita de evento sem abrir transação própria.
//!
//! Nenhuma delas abre transação: a atomicidade é a do `escrita` de quem chamou (um `SAVEPOINT` do
//! escritor), que volta inteiro no primeiro erro.

use poph_types::{
    AtorEvento, ConversationId, EventoFichaId, ExecucaoId, FechadaPor, FichaId, OrgId,
    StatusExecucao, StatusFicha, TipoEventoFicha,
};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::Store;
use crate::erro::{ErroStore, Resultado};
use crate::modelo::{EventoFicha, Ficha, FiltroFichas, NovaFicha, NovoEventoFicha, Pagina};
use crate::repos::{exigir, id_da_coluna, id_opcional, u64_da_coluna};

const COLUNAS: &str =
    "id, org_id, conversation_id, agent_id, canal, peer_externo, status, aberta_em,
    fechada_em, fechada_por, fechada_motivo, version";

const COLUNAS_EVENTO: &str = "id, org_id, ficha_id, execucao_id, seq, tipo, ator_tipo, ator_ref,
    payload_json, chave_idempotencia, created_at";

/// Motivo gravado quando o atendimento encerra e a execução de fluxo em andamento é abandonada
/// (decisão do mantenedor na FX-07: `abandonada` com este motivo, não `falha`).
pub const MOTIVO_ABANDONO_ATENDIMENTO: &str = "atendimento_encerrado";

/// Acesso às Fichas e aos eventos delas.
pub struct Fichas<'a>(pub(crate) &'a Store);

impl Fichas<'_> {
    /// Cria a Ficha aberta. O caminho normal não passa por aqui: a Ficha nasce dentro de
    /// [`super::Conversas::criar`] e da recepção do canal. Este método existe para quem precisa criar
    /// sem conversa (importação, cenário de teste) e para a superfície do repositório ficar completa.
    pub fn criar(&self, org: OrgId, nova: NovaFicha) -> Resultado<Ficha> {
        let id = FichaId::generate();
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let conversa = nova.conversation_id.map(|conversa| conversa.to_string());
        let agente = nova.agent_id.to_string();
        let canal = nova.canal;
        let peer = nova.peer_externo;
        self.0.escrita(move |conn| {
            conn.execute(
                "INSERT INTO fichas
                    (id, org_id, conversation_id, agent_id, canal, peer_externo, status, aberta_em,
                     fechada_em, fechada_por, fechada_motivo, version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'open', ?7, NULL, NULL, NULL, 1)",
                params![texto_id, texto_org, conversa, agente, canal, peer, agora],
            )
            .map_err(|erro| ErroStore::da_escrita("conversa", erro))?;
            registrar_evento_conn(
                conn,
                &texto_org,
                &texto_id,
                &NovoEventoFicha {
                    execucao_id: None,
                    tipo: TipoEventoFicha::FichaAberta,
                    ator_tipo: AtorEvento::Sistema,
                    ator_ref: None,
                    payload_json: "{}".to_owned(),
                    chave_idempotencia: None,
                },
                agora,
            )?;
            Ok(())
        })?;
        self.obter(org, id)
    }

    /// Busca pelo id dentro da org.
    pub fn obter(&self, org: OrgId, id: FichaId) -> Resultado<Ficha> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let encontrada = self.0.leitura(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUNAS} FROM fichas WHERE org_id = ?1 AND id = ?2"),
                params![texto_org, texto_id],
                linha,
            )
            .optional()
            .map_err(ErroStore::from)
        })?;
        exigir(encontrada, "ficha", &id.to_string())
    }

    /// Ficha da conversa, se houver.
    ///
    /// Devolve `None` em vez de erro porque "esta conversa tem Ficha?" é pergunta legítima de quem
    /// procura, embora a invariante diga que toda conversa criada pelo sistema tem uma.
    pub fn por_conversa(&self, org: OrgId, conversa: ConversationId) -> Resultado<Option<Ficha>> {
        let texto_org = org.to_string();
        let texto_conversa = conversa.to_string();
        self.0.leitura(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUNAS} FROM fichas WHERE org_id = ?1 AND conversation_id = ?2"),
                params![texto_org, texto_conversa],
                linha,
            )
            .optional()
            .map_err(ErroStore::from)
        })
    }

    /// Lista as Fichas da org com filtros, em ordem de id (UUID v7, ordenável por tempo) e paginada
    /// por cursor.
    ///
    /// A Ficha cuja conversa foi apagada pela retenção continua aparecendo (`conversation_id` nulo):
    /// é registro de negócio, não rascunho (PRD L12).
    pub fn listar(
        &self,
        org: OrgId,
        filtro: &FiltroFichas,
        pagina: Pagina,
    ) -> Resultado<Vec<Ficha>> {
        let mut condicoes = vec!["org_id = ?1".to_owned()];
        let mut valores: Vec<rusqlite::types::Value> =
            vec![rusqlite::types::Value::Text(org.to_string())];
        if let Some(status) = filtro.status {
            valores.push(rusqlite::types::Value::Text(status.como_texto().to_owned()));
            condicoes.push(format!("status = ?{}", valores.len()));
        }
        if let Some(agente) = filtro.agente {
            valores.push(rusqlite::types::Value::Text(agente.to_string()));
            condicoes.push(format!("agent_id = ?{}", valores.len()));
        }
        if let Some(canal) = &filtro.canal {
            valores.push(rusqlite::types::Value::Text(canal.clone()));
            condicoes.push(format!("canal = ?{}", valores.len()));
        }
        if let Some(desde) = filtro.desde {
            valores.push(rusqlite::types::Value::Integer(desde));
            condicoes.push(format!("aberta_em >= ?{}", valores.len()));
        }
        if let Some(apos) = pagina.apos {
            valores.push(rusqlite::types::Value::Text(apos.to_string()));
            condicoes.push(format!("id > ?{}", valores.len()));
        }
        valores.push(rusqlite::types::Value::Integer(i64::from(pagina.limite)));
        let sql = format!(
            "SELECT {COLUNAS} FROM fichas WHERE {} ORDER BY id LIMIT ?{}",
            condicoes.join(" AND "),
            valores.len()
        );
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(&sql)?;
            let linhas = consulta.query_map(rusqlite::params_from_iter(valores.iter()), linha)?;
            let mut fichas = Vec::new();
            for linha in linhas {
                fichas.push(linha?);
            }
            Ok(fichas)
        })
    }

    /// Eventos da Ficha em ordem de `seq`, paginados pelo próprio `seq` (a linha do tempo é lida na
    /// ordem em que aconteceu, não na ordem dos ids).
    ///
    /// Ficha de outra org (ou inexistente) → `NaoEncontrado`.
    pub fn eventos(
        &self,
        org: OrgId,
        ficha: FichaId,
        apos: Option<i64>,
        limite: u32,
    ) -> Resultado<Vec<EventoFicha>> {
        // Confere a Ficha antes: sem isso, ficha de outra org devolveria lista vazia em vez de 404.
        self.obter(org, ficha)?;
        let texto_org = org.to_string();
        let texto_ficha = ficha.to_string();
        let limite = i64::from(limite.clamp(1, 500));
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(&format!(
                "SELECT {COLUNAS_EVENTO} FROM ficha_eventos
                  WHERE org_id = ?1 AND ficha_id = ?2 AND (?3 IS NULL OR seq > ?3)
                  ORDER BY seq LIMIT ?4"
            ))?;
            let linhas =
                consulta.query_map(params![texto_org, texto_ficha, apos, limite], linha_evento)?;
            let mut eventos = Vec::new();
            for linha in linhas {
                eventos.push(linha?);
            }
            Ok(eventos)
        })
    }

    /// Fecha uma Ficha **sem conversa ligada** (FX-30): a conversa foi apagada pela retenção, então o
    /// fechamento da conversa não existe mais e a Ficha fecha sozinha, com as execuções ativas
    /// abandonadas e os eventos gravados, tudo numa escrita.
    ///
    /// `esperada` é a `version` da Ficha que o chamador leu (`If-Match`). Ordem das recusas, todas
    /// antes de gravar: Ficha de outra org ou inexistente → `404`; já fechada → `409`; com conversa
    /// ligada → `409` (quem fecha essa Ficha é a conversa, em `Conversas::fechar_auditado`); `version`
    /// diferente → `412`.
    pub fn fechar_sem_conversa(
        &self,
        org: OrgId,
        ficha: FichaId,
        esperada: u64,
        motivo: &str,
    ) -> Resultado<()> {
        let texto_org = org.to_string();
        let texto_ficha = ficha.to_string();
        let agora = self.0.agora();
        let motivo = motivo.to_owned();
        self.0.escrita(move |conn| {
            let linha: Option<(String, Option<String>, i64)> = conn
                .query_row(
                    "SELECT status, conversation_id, version FROM fichas
                      WHERE org_id = ?1 AND id = ?2",
                    params![texto_org, texto_ficha],
                    |linha| Ok((linha.get(0)?, linha.get(1)?, linha.get(2)?)),
                )
                .optional()
                .map_err(ErroStore::from)?;
            let Some((status, conversa, versao)) = linha else {
                return Err(ErroStore::Dominio(poph_types::DomainError::NaoEncontrado {
                    entidade: "ficha",
                    id: texto_ficha.clone(),
                }));
            };
            if StatusFicha::de_texto(&status).map_err(ErroStore::from)? == StatusFicha::Fechada {
                return Err(ErroStore::Dominio(poph_types::DomainError::FichaFechada {
                    id: texto_ficha.clone(),
                }));
            }
            if conversa.is_some() {
                return Err(ErroStore::Dominio(poph_types::DomainError::Conflito {
                    entidade: "ficha",
                    id: texto_ficha.clone(),
                    motivo: "a Ficha tem conversa ligada: feche o atendimento pela conversa"
                        .to_owned(),
                }));
            }
            let atual = u64::try_from(versao).unwrap_or(0);
            if atual != esperada {
                return Err(ErroStore::Dominio(
                    poph_types::DomainError::ConflitoDeVersao {
                        entidade: "ficha",
                        id: texto_ficha.clone(),
                        esperada,
                        atual,
                    },
                ));
            }
            fechar_ficha_por_id(
                conn,
                &texto_org,
                &texto_ficha,
                FechadaPor::Admin,
                &motivo,
                agora,
            )?;
            Ok(())
        })
    }

    /// Registra um evento na linha do tempo da Ficha e devolve o que ficou gravado.
    ///
    /// Idempotente por `chave_idempotencia`: repetir a mesma operação (reenvio, retomada depois de
    /// queda) devolve o evento já gravado, sem criar `seq` novo. A chave é conferida **antes** da
    /// situação da Ficha, para que repetir uma operação que já entrou não vire erro depois do
    /// fechamento. Ficha fechada recusa evento novo
    /// ([`poph_types::DomainError::FichaFechada`], PRD L5).
    pub fn registrar_evento(
        &self,
        org: OrgId,
        ficha: FichaId,
        novo: NovoEventoFicha<'_>,
    ) -> Resultado<EventoFicha> {
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let texto_ficha = ficha.to_string();
        let execucao = novo.execucao_id;
        let tipo = novo.tipo;
        let ator_tipo = novo.ator_tipo;
        let ator_ref = novo.ator_ref.map(str::to_owned);
        let payload_json = novo.payload_json;
        let chave = novo.chave_idempotencia.map(str::to_owned);
        let id = self.0.escrita(move |conn| {
            let situacao = situacao_da_ficha(conn, &texto_org, &texto_ficha)?;
            if situacao.is_none() {
                return Err(ErroStore::Dominio(poph_types::DomainError::NaoEncontrado {
                    entidade: "ficha",
                    id: texto_ficha.clone(),
                }));
            }
            if let Some(chave) = chave.as_deref()
                && let Some(evento) = evento_por_chave(conn, &texto_org, &texto_ficha, chave)?
            {
                return Ok(evento.id.to_string());
            }
            if situacao == Some(StatusFicha::Fechada) {
                return Err(ErroStore::Dominio(poph_types::DomainError::FichaFechada {
                    id: texto_ficha.clone(),
                }));
            }
            registrar_evento_conn(
                conn,
                &texto_org,
                &texto_ficha,
                &NovoEventoFicha {
                    execucao_id: execucao,
                    tipo,
                    ator_tipo,
                    ator_ref: ator_ref.as_deref(),
                    payload_json,
                    chave_idempotencia: chave.as_deref(),
                },
                agora,
            )
        })?;
        let texto_org = org.to_string();
        let evento_id: EventoFichaId = id
            .parse()
            .map_err(|erro: poph_types::DomainError| ErroStore::Dominio(erro))?;
        self.0.leitura(move |conn| {
            conn.query_row(
                &format!(
                    "SELECT {COLUNAS_EVENTO} FROM ficha_eventos WHERE org_id = ?1 AND id = ?2"
                ),
                params![texto_org, evento_id.to_string()],
                linha_evento,
            )
            .optional()
            .map_err(ErroStore::from)?
            .ok_or_else(|| {
                ErroStore::Interno("o evento foi gravado mas não voltou na leitura".to_owned())
            })
        })
    }
}

/// Situação da Ficha na org, ou `None` quando ela não existe.
fn situacao_da_ficha(conn: &Connection, org: &str, ficha: &str) -> Resultado<Option<StatusFicha>> {
    let texto: Option<String> = conn
        .query_row(
            "SELECT status FROM fichas WHERE org_id = ?1 AND id = ?2",
            params![org, ficha],
            |linha| linha.get(0),
        )
        .optional()
        .map_err(ErroStore::from)?;
    texto
        .map(|texto| StatusFicha::de_texto(&texto).map_err(ErroStore::from))
        .transpose()
}

/// Evento já gravado com a chave de idempotência dada.
pub(crate) fn evento_por_chave(
    conn: &Connection,
    org: &str,
    ficha: &str,
    chave: &str,
) -> Resultado<Option<EventoFicha>> {
    conn.query_row(
        &format!(
            "SELECT {COLUNAS_EVENTO} FROM ficha_eventos
              WHERE org_id = ?1 AND ficha_id = ?2 AND chave_idempotencia = ?3"
        ),
        params![org, ficha, chave],
        linha_evento,
    )
    .optional()
    .map_err(ErroStore::from)
}

/// Eventos da Ficha com `seq` maior que `apos`, em ordem de `seq`, dentro da escrita de quem
/// chamou. É como a operação atômica de transição (FX-20) devolve o que acabou de gravar sem
/// recalcular `seq` por fora.
pub(crate) fn ler_eventos_conn(
    conn: &Connection,
    org: &str,
    ficha: &str,
    apos: i64,
) -> Resultado<Vec<EventoFicha>> {
    let mut consulta = conn
        .prepare(&format!(
            "SELECT {COLUNAS_EVENTO} FROM ficha_eventos
              WHERE org_id = ?1 AND ficha_id = ?2 AND seq > ?3
              ORDER BY seq"
        ))
        .map_err(ErroStore::from)?;
    let linhas = consulta
        .query_map(params![org, ficha, apos], linha_evento)
        .map_err(ErroStore::from)?;
    let mut eventos = Vec::new();
    for linha in linhas {
        eventos.push(linha.map_err(ErroStore::from)?);
    }
    Ok(eventos)
}

/// `seq` mais alto da Ficha (`0` quando a linha do tempo está vazia).
pub(crate) fn ultimo_seq(conn: &Connection, org: &str, ficha: &str) -> Resultado<i64> {
    conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) FROM ficha_eventos WHERE org_id = ?1 AND ficha_id = ?2",
        params![org, ficha],
        |linha| linha.get(0),
    )
    .map_err(ErroStore::from)
}

/// Cria a Ficha aberta de uma conversa **dentro da escrita de quem chamou** (PRD L4).
///
/// Devolve o id da Ficha criada. Não abre transação: roda no `SAVEPOINT` do chamador, então conversa
/// e Ficha entram juntas ou não entram. Grava o evento `ficha_aberta` no mesmo passo, para a linha do
/// tempo de um atendimento novo não começar vazia.
pub(crate) fn criar_ficha_da_conversa(
    conn: &Connection,
    org: &str,
    conversa: &str,
    agent_id: &str,
    canal: &str,
    peer: Option<&str>,
    agora: i64,
) -> Resultado<String> {
    let ficha = FichaId::generate().to_string();
    conn.execute(
        "INSERT INTO fichas
            (id, org_id, conversation_id, agent_id, canal, peer_externo, status, aberta_em,
             fechada_em, fechada_por, fechada_motivo, version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'open', ?7, NULL, NULL, NULL, 1)",
        params![ficha, org, conversa, agent_id, canal, peer, agora],
    )
    .map_err(|erro| ErroStore::da_escrita("conversa", erro))?;
    registrar_evento_conn(
        conn,
        org,
        &ficha,
        &NovoEventoFicha {
            execucao_id: None,
            tipo: TipoEventoFicha::FichaAberta,
            ator_tipo: AtorEvento::Sistema,
            ator_ref: None,
            payload_json: "{}".to_owned(),
            chave_idempotencia: None,
        },
        agora,
    )?;
    Ok(ficha)
}

/// Fecha a Ficha da conversa **dentro da escrita de quem chamou** (PRD L5 e L6): marca a Ficha
/// fechada, abandona a execução de fluxo ativa com o motivo `atendimento_encerrado` e registra os
/// eventos, tudo no mesmo passo.
///
/// Devolve `true` quando fechou agora e `false` quando a Ficha já estava fechada (ou a conversa já
/// não existia) — fechar duas vezes não cria evento nem `seq` repetido.
///
/// Se a conversa não tinha Ficha (banco que escapou do backfill da 0012), a Ficha nasce fechada a
/// partir dos dados da conversa: a invariante fica valendo em vez de o fechamento passar em silêncio.
pub(crate) fn fechar_ficha_da_conversa(
    conn: &Connection,
    org: &str,
    conversa: &str,
    por: FechadaPor,
    motivo: &str,
    agora: i64,
) -> Resultado<bool> {
    let situacao: Option<(String, String)> = conn
        .query_row(
            "SELECT id, status FROM fichas WHERE org_id = ?1 AND conversation_id = ?2",
            params![org, conversa],
            |linha| Ok((linha.get(0)?, linha.get(1)?)),
        )
        .optional()
        .map_err(ErroStore::from)?;
    let ficha = match situacao {
        Some((id, status)) => {
            if StatusFicha::de_texto(&status).map_err(ErroStore::from)? == StatusFicha::Fechada {
                return Ok(false);
            }
            id
        }
        None => {
            let ficha = FichaId::generate().to_string();
            let inseridas = conn
                .execute(
                    "INSERT INTO fichas
                        (id, org_id, conversation_id, agent_id, canal, peer_externo, status,
                         aberta_em, fechada_em, fechada_por, fechada_motivo, version)
                     SELECT ?1, org_id, id, agent_id, channel, external_peer, 'closed', created_at,
                            ?3, ?4, ?5, 1
                       FROM conversations WHERE org_id = ?2 AND id = ?6",
                    params![ficha, org, agora, por.como_texto(), motivo, conversa],
                )
                .map_err(|erro| ErroStore::da_escrita("conversa", erro))?;
            if inseridas == 0 {
                return Ok(false);
            }
            registrar_evento_conn(conn, org, &ficha, &evento_de_fechamento(por, motivo), agora)?;
            return Ok(true);
        }
    };

    // Daqui para baixo o fechamento é um só: o mesmo de quem tem a Ficha pelo id (FX-20).
    fechar_ficha_por_id(conn, org, &ficha, por, motivo, agora)
}

/// Fecha a Ficha **pelo id dela**, dentro da escrita de quem chamou: abandona as execuções de fluxo
/// ativas com o motivo `atendimento_encerrado`, registra os eventos e marca a Ficha `closed`.
///
/// Devolve `true` quando fechou agora e `false` quando já estava fechada. Existe separado de
/// [`fechar_ficha_da_conversa`] porque a operação atômica de transição (FX-20) conhece a **Ficha**
/// (a execução aponta para ela) e não a conversa, que pode ter sido apagada pela retenção.
pub(crate) fn fechar_ficha_por_id(
    conn: &Connection,
    org: &str,
    ficha: &str,
    por: FechadaPor,
    motivo: &str,
    agora: i64,
) -> Resultado<bool> {
    let situacao: Option<String> = conn
        .query_row(
            "SELECT status FROM fichas WHERE org_id = ?1 AND id = ?2",
            params![org, ficha],
            |linha| linha.get(0),
        )
        .optional()
        .map_err(ErroStore::from)?;
    let Some(situacao) = situacao else {
        return Err(ErroStore::Dominio(poph_types::DomainError::NaoEncontrado {
            entidade: "ficha",
            id: ficha.to_owned(),
        }));
    };
    if StatusFicha::de_texto(&situacao).map_err(ErroStore::from)? == StatusFicha::Fechada {
        return Ok(false);
    }

    // Execuções ativas da Ficha: abandonadas na mesma transação (PRD L5 e L6).
    let ativas = execucoes_ativas(conn, org, ficha)?;
    let abandonada = StatusExecucao::Abandonada.como_texto();
    for execucao in &ativas {
        conn.execute(
            "UPDATE fluxo_execucoes
                SET status = ?3, motivo_fim = ?4, finalizada_em = ?5, version = version + 1
              WHERE org_id = ?1 AND id = ?2",
            params![
                org,
                execucao,
                abandonada,
                MOTIVO_ABANDONO_ATENDIMENTO,
                agora
            ],
        )
        .map_err(ErroStore::from)?;
        registrar_evento_conn(
            conn,
            org,
            ficha,
            &NovoEventoFicha {
                execucao_id: execucao.parse::<ExecucaoId>().ok(),
                tipo: TipoEventoFicha::ExecucaoAbandonada,
                ator_tipo: AtorEvento::Sistema,
                ator_ref: None,
                payload_json: format!(
                    "{{\"motivo\":{}}}",
                    json_string(MOTIVO_ABANDONO_ATENDIMENTO)
                ),
                chave_idempotencia: None,
            },
            agora,
        )?;
    }

    registrar_evento_conn(conn, org, ficha, &evento_de_fechamento(por, motivo), agora)?;

    let alteradas = conn
        .execute(
            "UPDATE fichas
                SET status = 'closed', fechada_em = ?3, fechada_por = ?4, fechada_motivo = ?5,
                    version = version + 1
              WHERE org_id = ?1 AND id = ?2 AND status = 'open'",
            params![org, ficha, agora, por.como_texto(), motivo],
        )
        .map_err(|erro| ErroStore::da_escrita("fechada_por", erro))?;
    Ok(alteradas > 0)
}

/// Evento `ficha_fechada`, com o autor do fechamento traduzido para o ator do evento.
fn evento_de_fechamento(por: FechadaPor, motivo: &str) -> NovoEventoFicha<'static> {
    NovoEventoFicha {
        execucao_id: None,
        tipo: TipoEventoFicha::FichaFechada,
        ator_tipo: match por {
            FechadaPor::Admin => AtorEvento::Admin,
            FechadaPor::Agente => AtorEvento::Agente,
            FechadaPor::Atendente => AtorEvento::Atendente,
            FechadaPor::Cliente
            | FechadaPor::Fluxo
            | FechadaPor::Sistema
            | FechadaPor::Migracao => AtorEvento::Sistema,
        },
        ator_ref: None,
        payload_json: format!("{{\"motivo\":{}}}", json_string(motivo)),
        chave_idempotencia: None,
    }
}

/// Ids das execuções `ativa` da Ficha (no máximo uma, pelo índice único parcial).
fn execucoes_ativas(conn: &Connection, org: &str, ficha: &str) -> Resultado<Vec<String>> {
    let mut consulta = conn
        .prepare(
            "SELECT id FROM fluxo_execucoes
              WHERE org_id = ?1 AND ficha_id = ?2 AND status = 'ativa'",
        )
        .map_err(ErroStore::from)?;
    let linhas = consulta
        .query_map(params![org, ficha], |linha| linha.get::<_, String>(0))
        .map_err(ErroStore::from)?;
    let mut ids = Vec::new();
    for linha in linhas {
        ids.push(linha.map_err(ErroStore::from)?);
    }
    Ok(ids)
}

/// Escapa um texto para entrar num JSON de payload. O payload daqui carrega um motivo curto e
/// conhecido; não é preciso um serializador para isso.
fn json_string(texto: &str) -> String {
    let mut saida = String::with_capacity(texto.len() + 2);
    saida.push('"');
    for caractere in texto.chars() {
        match caractere {
            '"' => saida.push_str("\\\""),
            '\\' => saida.push_str("\\\\"),
            '\n' => saida.push_str("\\n"),
            '\r' => saida.push_str("\\r"),
            '\t' => saida.push_str("\\t"),
            outro => saida.push(outro),
        }
    }
    saida.push('"');
    saida
}

/// Grava um evento **sem abrir transação** e devolve o id dele. Chamado pelos caminhos que já estão
/// dentro de uma escrita (criação e fechamento da Ficha).
///
/// `seq` é `MAX(seq) + 1` da Ficha, calculado na mesma escrita: como só o escritor dedicado grava,
/// duas chamadas não leem o mesmo máximo. O índice único `(ficha_id, seq)` é a segunda linha de
/// defesa.
pub(crate) fn registrar_evento_conn(
    conn: &Connection,
    org: &str,
    ficha: &str,
    novo: &NovoEventoFicha<'_>,
    agora: i64,
) -> Resultado<String> {
    if let Some(execucao) = novo.execucao_id {
        // `ficha_eventos.execucao_id` ficou sem chave estrangeira (decisão do mantenedor): o repositório
        // confere que a execução existe **nesta org** e que ela é **desta Ficha** antes de gravar —
        // sem a segunda parte, um evento poderia apontar para a execução de outro atendimento da
        // mesma org.
        pertence_a_ficha(conn, org, ficha, &execucao.to_string())?;
    }
    let id = EventoFichaId::generate().to_string();
    conn.execute(
        "INSERT INTO ficha_eventos
            (id, org_id, ficha_id, execucao_id, seq, tipo, ator_tipo, ator_ref, payload_json,
             chave_idempotencia, created_at)
         VALUES (?1, ?2, ?3, ?4,
                 (SELECT COALESCE(MAX(seq), 0) + 1 FROM ficha_eventos
                   WHERE org_id = ?2 AND ficha_id = ?3),
                 ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            id,
            org,
            ficha,
            novo.execucao_id.map(|execucao| execucao.to_string()),
            novo.tipo.como_texto(),
            novo.ator_tipo.como_texto(),
            novo.ator_ref,
            novo.payload_json,
            novo.chave_idempotencia,
            agora
        ],
    )
    .map_err(|erro| ErroStore::da_escrita("tipo", erro))?;
    Ok(id)
}

/// Confere que a execução de fluxo existe nesta org **e pertence à Ficha**.
fn pertence_a_ficha(conn: &Connection, org: &str, ficha: &str, execucao: &str) -> Resultado<()> {
    let existe: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM fluxo_execucoes WHERE org_id = ?1 AND ficha_id = ?2 AND id = ?3",
            params![org, ficha, execucao],
            |linha| linha.get(0),
        )
        .optional()
        .map_err(ErroStore::from)?;
    match existe {
        Some(_) => Ok(()),
        None => Err(ErroStore::Dominio(poph_types::DomainError::NaoEncontrado {
            entidade: "execução de fluxo",
            id: execucao.to_owned(),
        })),
    }
}

/// Lê uma Ficha da linha do resultado.
fn linha(linha: &Row<'_>) -> rusqlite::Result<Ficha> {
    Ok(Ficha {
        id: id_da_coluna(linha, 0)?,
        org_id: id_da_coluna(linha, 1)?,
        conversation_id: id_opcional(linha, 2)?,
        agent_id: id_da_coluna(linha, 3)?,
        canal: linha.get(4)?,
        peer_externo: linha.get(5)?,
        status: StatusFicha::de_texto(&linha.get::<_, String>(6)?)
            .map_err(|erro| conversao(6, erro))?,
        aberta_em: linha.get(7)?,
        fechada_em: linha.get(8)?,
        fechada_por: match linha.get::<_, Option<String>>(9)? {
            Some(texto) => Some(FechadaPor::de_texto(&texto).map_err(|erro| conversao(9, erro))?),
            None => None,
        },
        fechada_motivo: linha.get(10)?,
        versao: u64_da_coluna(linha, 11)?,
    })
}

/// Lê um evento da linha do resultado.
fn linha_evento(linha: &Row<'_>) -> rusqlite::Result<EventoFicha> {
    Ok(EventoFicha {
        id: id_da_coluna(linha, 0)?,
        org_id: id_da_coluna(linha, 1)?,
        ficha_id: id_da_coluna(linha, 2)?,
        execucao_id: id_opcional(linha, 3)?,
        seq: linha.get(4)?,
        tipo: TipoEventoFicha::de_texto(&linha.get::<_, String>(5)?)
            .map_err(|erro| conversao(5, erro))?,
        ator_tipo: AtorEvento::de_texto(&linha.get::<_, String>(6)?)
            .map_err(|erro| conversao(6, erro))?,
        ator_ref: linha.get(7)?,
        payload_json: linha.get(8)?,
        chave_idempotencia: linha.get(9)?,
        criado_em: linha.get(10)?,
    })
}

/// Texto de coluna que não é valor do vocabulário → erro de conversão do SQLite.
fn conversao(indice: usize, erro: poph_types::DomainError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(indice, rusqlite::types::Type::Text, Box::new(erro))
}
