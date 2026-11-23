//! Repositório de turnos e de auditoria.
//!
//! O turno é o registro do "por quê" (PRD §6 passo 9): decisões do Jev, trechos recuperados,
//! ferramentas, tempos, tokens e custo. A auditoria guarda quem mudou o quê (PRD §7.2).

use poph_types::{AgentId, AuditId, ConversationId, OrgId, TurnId};
use rusqlite::{OptionalExtension, Row, params};

use crate::Store;
use crate::erro::{ErroStore, Resultado};
use crate::modelo::{
    EstadoTurno, FinalizacaoTurno, NovoRegistroAuditoria, Pagina, RegistroAuditoria, Turno,
};
use crate::repos::{id_da_coluna, id_opcional, pertence};

const COLUNAS_TURNO: &str = "id, org_id, conversation_id, agent_id, state, jev_json,
    retrieved_json, tools_json, timings_json, input_tokens, output_tokens, cost_micros, error,
    started_at, finished_at";

const COLUNAS_AUDITORIA: &str =
    "id, org_id, actor_kind, actor_id, action, entity, entity_id, detail_json, created_at";

/// Acesso aos turnos.
pub struct Turnos<'a>(pub(crate) &'a Store);

impl Turnos<'_> {
    /// Abre o turno (`running`).
    pub fn iniciar(
        &self,
        org: OrgId,
        conversa: ConversationId,
        agente: Option<AgentId>,
    ) -> Resultado<Turno> {
        let id = TurnId::generate();
        let agora = self.0.agora();
        let texto_org = org.to_string();
        self.0.escrita(move |conn| {
            if let Some(agente) = agente {
                pertence(conn, "agents", "agente", &texto_org, &agente.to_string())?;
            }
            conn.execute(
                "INSERT INTO turns
                    (id, org_id, conversation_id, agent_id, state, jev_json, retrieved_json,
                     tools_json, timings_json, input_tokens, output_tokens, cost_micros, error,
                     started_at, finished_at)
                 VALUES (?1, ?2, ?3, ?4, 'running', '{}', '[]', '[]', '{}', 0, 0, 0, NULL, ?5, NULL)",
                params![
                    id.to_string(),
                    texto_org,
                    conversa.to_string(),
                    agente.map(|agente| agente.to_string()),
                    agora
                ],
            )
            .map_err(ErroStore::from)?;
            Ok(())
        })?;
        self.obter(org, id)
    }

    /// Fecha o turno com o que foi medido (estado, JSONs, tokens, custo, erro).
    pub fn finalizar(&self, org: OrgId, id: TurnId, fim: FinalizacaoTurno) -> Resultado<Turno> {
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let estado = fim.estado.map(EstadoTurno::como_texto);
        let efeito = self.0.escrita(move |conn| {
            conn.execute(
                "UPDATE turns
                    SET state = COALESCE(?3, state),
                        jev_json = COALESCE(?4, jev_json),
                        retrieved_json = COALESCE(?5, retrieved_json),
                        tools_json = COALESCE(?6, tools_json),
                        timings_json = COALESCE(?7, timings_json),
                        input_tokens = COALESCE(?8, input_tokens),
                        output_tokens = COALESCE(?9, output_tokens),
                        cost_micros = COALESCE(?10, cost_micros),
                        error = ?11,
                        finished_at = ?12
                  WHERE org_id = ?1 AND id = ?2",
                params![
                    texto_org,
                    texto_id,
                    estado,
                    fim.jev_json,
                    fim.trechos_json,
                    fim.ferramentas_json,
                    fim.tempos_json,
                    fim.tokens_entrada.map(i64::from),
                    fim.tokens_saida.map(i64::from),
                    fim.custo_micros,
                    fim.erro,
                    agora
                ],
            )
            .map_err(ErroStore::from)
        })?;
        if efeito == 0 {
            return Err(ErroStore::Dominio(poph_types::DomainError::NaoEncontrado {
                entidade: "turno",
                id: id.to_string(),
            }));
        }
        self.obter(org, id)
    }

    /// Fecha como falho o turno que ficou `running` de um processo anterior (HR-57).
    ///
    /// Queda do processo no meio do atendimento deixa a linha do turno em `running` para sempre: o
    /// registro do "por quê" mentiria que o atendimento ainda está em curso. A subida do servidor
    /// chama isto com a hora da subida; só são tocados turnos **começados até** ela (o carimbo tem
    /// resolução de milissegundo), então um turno aberto depois da subida nesta instância nunca é
    /// afetado. O erro gravado é curto e não tem dado do cliente.
    ///
    /// Devolve quantos turnos foram fechados.
    pub fn falhar_orfaos(&self, org: OrgId, motivo: &str, antes_de: i64) -> Resultado<u64> {
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let motivo = motivo.to_owned();
        let efeito = self.0.escrita(move |conn| {
            conn.execute(
                "UPDATE turns
                    SET state = 'failed', error = ?3, finished_at = ?4
                  WHERE org_id = ?1 AND state = 'running' AND started_at <= ?2",
                params![texto_org, antes_de, motivo, agora],
            )
            .map_err(ErroStore::from)
        })?;
        Ok(u64::try_from(efeito).unwrap_or(u64::MAX))
    }

    /// Busca pelo id dentro da org.
    pub fn obter(&self, org: OrgId, id: TurnId) -> Resultado<Turno> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let encontrado = self.0.leitura(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUNAS_TURNO} FROM turns WHERE org_id = ?1 AND id = ?2"),
                params![texto_org, texto_id],
                linha_turno,
            )
            .optional()
            .map_err(ErroStore::from)
        })?;
        super::exigir(encontrado, "turno", &id.to_string())
    }

    /// Há turno **em andamento** nesta conversa agora?
    ///
    /// O efeito `mensagem_ao_cliente` só entra na `outbox` depois da resposta do turno (PRD §6.8):
    /// enquanto este método disser `true`, o entregador espera. É leitura barata e sem índice novo
    /// (o caminho é `conversation_id`, já indexado pela fila de entrada).
    pub fn em_andamento(&self, org: OrgId, conversa: ConversationId) -> Resultado<bool> {
        let texto_org = org.to_string();
        let texto_conversa = conversa.to_string();
        self.0.leitura(move |conn| {
            conn.query_row(
                "SELECT 1 FROM turns
                  WHERE org_id = ?1 AND conversation_id = ?2 AND state = 'running'
                  LIMIT 1",
                params![texto_org, texto_conversa],
                |linha| linha.get::<_, i64>(0),
            )
            .optional()
            .map(|achado| achado.is_some())
            .map_err(ErroStore::from)
        })
    }

    /// Lista os turnos de uma conversa, do mais antigo para o mais novo.
    pub fn listar(
        &self,
        org: OrgId,
        conversa: ConversationId,
        pagina: Pagina,
    ) -> Resultado<Vec<Turno>> {
        let texto_org = org.to_string();
        let texto_conversa = conversa.to_string();
        let apos = pagina.apos.map(|id| id.to_string());
        let limite = i64::from(pagina.limite);
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(&format!(
                "SELECT {COLUNAS_TURNO} FROM turns
                  WHERE org_id = ?1 AND conversation_id = ?2 AND (?3 IS NULL OR id > ?3)
                  ORDER BY id LIMIT ?4"
            ))?;
            let linhas = consulta.query_map(
                params![texto_org, texto_conversa, apos, limite],
                linha_turno,
            )?;
            let mut turnos = Vec::new();
            for linha in linhas {
                turnos.push(linha?);
            }
            Ok(turnos)
        })
    }
}

/// Acesso à auditoria.
pub struct Auditoria<'a>(pub(crate) &'a Store);

impl Auditoria<'_> {
    /// Lista os eventos globais, inclusive exclusões de organizações.
    pub fn listar_globais(&self, pagina: Pagina) -> Resultado<Vec<RegistroAuditoria>> {
        let apos = pagina.apos.map(|id| id.to_string());
        let limite = i64::from(pagina.limite);
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(&format!(
                "SELECT {COLUNAS_AUDITORIA} FROM audit_log WHERE org_id IS NULL AND (?1 IS NULL OR id < ?1) ORDER BY id DESC LIMIT ?2"
            ))?;
            let linhas = consulta.query_map(params![apos, limite], linha_auditoria)?;
            let mut registros = Vec::new();
            for linha in linhas { registros.push(linha?); }
            Ok(registros)
        })
    }

    /// Registra o evento. `org = None` é ação global (token global).
    pub fn registrar(
        &self,
        org: Option<OrgId>,
        novo: NovoRegistroAuditoria,
    ) -> Resultado<RegistroAuditoria> {
        let registro = RegistroAuditoria {
            id: AuditId::generate(),
            org_id: org,
            ator_tipo: novo.ator_tipo,
            ator_id: novo.ator_id,
            acao: novo.acao,
            entidade: novo.entidade,
            entidade_id: novo.entidade_id,
            detalhe_json: novo.detalhe_json,
            criado_em: self.0.agora(),
        };
        let para_gravar = registro.clone();
        self.0.escrita(move |conn| {
            conn.execute(
                "INSERT INTO audit_log
                    (id, org_id, actor_kind, actor_id, action, entity, entity_id, detail_json,
                     created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    para_gravar.id.to_string(),
                    para_gravar.org_id.map(|org| org.to_string()),
                    para_gravar.ator_tipo,
                    para_gravar.ator_id,
                    para_gravar.acao,
                    para_gravar.entidade,
                    para_gravar.entidade_id,
                    para_gravar.detalhe_json,
                    para_gravar.criado_em
                ],
            )
            .map_err(ErroStore::from)?;
            Ok(())
        })?;
        Ok(registro)
    }

    /// Lista a auditoria da org, mais recente primeiro.
    pub fn listar(&self, org: OrgId, pagina: Pagina) -> Resultado<Vec<RegistroAuditoria>> {
        let texto_org = org.to_string();
        let apos = pagina.apos.map(|id| id.to_string());
        let limite = i64::from(pagina.limite);
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(&format!(
                "SELECT {COLUNAS_AUDITORIA} FROM audit_log
                  WHERE org_id = ?1 AND (?2 IS NULL OR id < ?2)
                  ORDER BY id DESC LIMIT ?3"
            ))?;
            let linhas = consulta.query_map(params![texto_org, apos, limite], linha_auditoria)?;
            let mut registros = Vec::new();
            for linha in linhas {
                registros.push(linha?);
            }
            Ok(registros)
        })
    }
}

fn linha_turno(linha: &Row<'_>) -> rusqlite::Result<Turno> {
    Ok(Turno {
        id: id_da_coluna(linha, 0)?,
        org_id: id_da_coluna(linha, 1)?,
        conversation_id: id_da_coluna(linha, 2)?,
        agent_id: id_opcional(linha, 3)?,
        estado: EstadoTurno::de_texto(&linha.get::<_, String>(4)?).map_err(|erro| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Text,
                Box::new(erro),
            )
        })?,
        jev_json: linha.get(5)?,
        trechos_json: linha.get(6)?,
        ferramentas_json: linha.get(7)?,
        tempos_json: linha.get(8)?,
        tokens_entrada: linha.get(9)?,
        tokens_saida: linha.get(10)?,
        custo_micros: linha.get(11)?,
        erro: linha.get(12)?,
        iniciado_em: linha.get(13)?,
        finalizado_em: linha.get(14)?,
    })
}

fn linha_auditoria(linha: &Row<'_>) -> rusqlite::Result<RegistroAuditoria> {
    Ok(RegistroAuditoria {
        id: id_da_coluna(linha, 0)?,
        org_id: id_opcional(linha, 1)?,
        ator_tipo: linha.get(2)?,
        ator_id: linha.get(3)?,
        acao: linha.get(4)?,
        entidade: linha.get(5)?,
        entidade_id: linha.get(6)?,
        detalhe_json: linha.get(7)?,
        criado_em: linha.get(8)?,
    })
}
