//! Repositório de retenção (HR-51): a varredura que apaga o histórico vencido de uma org.
//!
//! Regra de escopo: a função só apaga o que está na lista abaixo. Base de conhecimento, produtos,
//! biblioteca de arquivos, orgs, agentes, tokens, departamentos, categorias e `audit_log` nunca
//! entram aqui.
//!
//! O que sai (tudo com `org_id` da org varrida e data anterior ao corte):
//! - mensagens e os anexos delas;
//! - linhas de `outbox` já enviadas;
//! - linhas de `inbox` já processadas;
//! - turnos encerrados (fora `running`), sem saída pendente e sem mensagem que aponte para eles;
//! - conversas vazias (sem mensagem, turno, saída nem carrinho) e criadas antes do corte.
//!
//! O que fica: pendente de inbox e de outbox (qualquer estado que não seja `sent`), turno
//! `running`, e qualquer conversa com algo que ainda não venceu.
//!
//! A operação roda dentro do escritor, que já abre a transação (`BEGIN IMMEDIATE`) do lote.
//! Cada varredura de org é um `SAVEPOINT` próprio: com `simular`, ele é desfeito no fim, e as
//! contagens devolvidas são as que sairiam de verdade.

use std::collections::BTreeSet;

use poph_types::OrgId;
use rusqlite::{Connection, params};

use crate::Store;
use crate::error::{Resultado, StoreError};

const SAVEPOINT: &str = "poph_retencao";

/// O que a varredura apagou (ou apagaria, na simulação) numa org.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VarreduraRetencao {
    /// Mensagens além da retenção.
    pub mensagens: u64,
    /// Anexos que saíram junto com as mensagens.
    pub anexos: u64,
    /// Turnos encerrados antes do corte.
    pub turnos: u64,
    /// Entradas já processadas antes do corte.
    pub inbox: u64,
    /// Saídas já enviadas antes do corte.
    pub outbox: u64,
    /// Conversas vazias e antigas removidas.
    pub conversas: u64,
    /// Ids (ordenados) das conversas afetadas: mensagem apagada ou conversa removida.
    pub ids_conversas: Vec<String>,
    /// sha256 que continuam com dono depois da varredura (anexos vivos e biblioteca do agente).
    pub hashes_referenciados: Vec<String>,
}

/// Acesso à retenção.
pub struct Retencao<'a>(pub(crate) &'a Store);

impl Retencao<'_> {
    /// Varre a org até `corte_ms` (ms desde a época Unix). Com `simular = true`, conta e desfaz.
    ///
    /// Erro de banco não deixa nada pela metade: a varredura inteira está num savepoint.
    pub fn varrer(&self, org: OrgId, corte_ms: i64, simular: bool) -> Resultado<VarreduraRetencao> {
        let texto = org.to_string();
        self.0.write(move |conn| {
            conn.execute_batch(&format!("SAVEPOINT {SAVEPOINT}"))?;
            let resultado = varrer_na_transacao(conn, &texto, corte_ms);
            let fechamento = if resultado.is_ok() && !simular {
                format!("RELEASE {SAVEPOINT}")
            } else {
                format!("ROLLBACK TO {SAVEPOINT}; RELEASE {SAVEPOINT}")
            };
            conn.execute_batch(&fechamento).map_err(StoreError::from)?;
            resultado
        })
    }
}

fn varrer_na_transacao(conn: &Connection, org: &str, corte: i64) -> Resultado<VarreduraRetencao> {
    let mut varredura = VarreduraRetencao::default();
    let mut conversas: BTreeSet<String> = BTreeSet::new();

    // 1. Conversas das mensagens que vão sair (antes de apagar, para não perder a referência).
    {
        let mut consulta = conn.prepare(
            "SELECT DISTINCT conversation_id FROM messages
             WHERE org_id = ?1 AND created_at < ?2",
        )?;
        let linhas = consulta.query_map(params![org, corte], |linha| linha.get::<_, String>(0))?;
        for linha in linhas {
            conversas.insert(linha?);
        }
    }

    // 2. Anexos primeiro (contagem exata), depois as mensagens.
    varredura.anexos = conn.execute(
        "DELETE FROM attachments
          WHERE org_id = ?1
            AND message_id IN (SELECT id FROM messages WHERE org_id = ?1 AND created_at < ?2)",
        params![org, corte],
    )? as u64;
    varredura.mensagens = conn.execute(
        "DELETE FROM messages WHERE org_id = ?1 AND created_at < ?2",
        params![org, corte],
    )? as u64;

    // 3. Saída já enviada. Pendente, enviando ou com falha fica.
    varredura.outbox = conn.execute(
        "DELETE FROM outbox
          WHERE org_id = ?1 AND state = 'sent' AND COALESCE(sent_at, created_at) < ?2",
        params![org, corte],
    )? as u64;

    // 4. Entrada já processada. Não processada (pendente ou com falha) fica.
    varredura.inbox = conn.execute(
        "DELETE FROM inbox
          WHERE org_id = ?1 AND processed_at IS NOT NULL AND processed_at < ?2",
        params![org, corte],
    )? as u64;

    // 5. Turnos encerrados, sem saída nem mensagem que ainda aponte para eles.
    varredura.turnos = conn.execute(
        "DELETE FROM turns
          WHERE org_id = ?1
            AND state <> 'running'
            AND COALESCE(finished_at, started_at) < ?2
            AND NOT EXISTS (SELECT 1 FROM outbox o WHERE o.turn_id = turns.id)
            AND NOT EXISTS (SELECT 1 FROM messages m WHERE m.turn_id = turns.id)",
        params![org, corte],
    )? as u64;

    // 6. Conversas vazias e criadas antes do corte.
    const VAZIA: &str = "NOT EXISTS (SELECT 1 FROM messages m
                           WHERE m.org_id = conversations.org_id AND m.conversation_id = conversations.id)
        AND NOT EXISTS (SELECT 1 FROM turns t
                           WHERE t.org_id = conversations.org_id AND t.conversation_id = conversations.id)
        AND NOT EXISTS (SELECT 1 FROM outbox o
                           WHERE o.org_id = conversations.org_id AND o.conversation_id = conversations.id)
        AND NOT EXISTS (SELECT 1 FROM carts k
                           WHERE k.org_id = conversations.org_id AND k.conversation_id = conversations.id)";
    {
        let mut consulta = conn.prepare(&format!(
            "SELECT id FROM conversations
              WHERE org_id = ?1 AND created_at < ?2 AND {VAZIA}"
        ))?;
        let linhas = consulta.query_map(params![org, corte], |linha| linha.get::<_, String>(0))?;
        for linha in linhas {
            conversas.insert(linha?);
        }
    }
    varredura.conversas = conn.execute(
        &format!(
            "DELETE FROM conversations
              WHERE org_id = ?1 AND created_at < ?2 AND {VAZIA}"
        ),
        params![org, corte],
    )? as u64;

    // 7. sha256 que ainda têm dono nesta org (depois de tudo apagado).
    {
        let mut consulta = conn.prepare(
            "SELECT sha256 FROM attachments WHERE org_id = ?1
             UNION
             SELECT sha256 FROM file_library WHERE org_id = ?1
             ORDER BY 1",
        )?;
        let linhas = consulta.query_map(params![org], |linha| linha.get::<_, String>(0))?;
        for linha in linhas {
            varredura.hashes_referenciados.push(linha?);
        }
    }

    varredura.ids_conversas = conversas.into_iter().collect();
    Ok(varredura)
}
