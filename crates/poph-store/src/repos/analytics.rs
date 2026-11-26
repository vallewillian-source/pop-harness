use poph_types::OrgId;
use rusqlite::params;

use crate::Store;
use crate::erro::{ErroStore, Resultado};

#[derive(Debug, Clone, PartialEq)]
pub struct NodeCount {
    pub node_id: String,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OperationalSummary {
    pub open_fichas: u64,
    pub closed_fichas: u64,
    pub open_conversations: u64,
    pub avg_resolution_ms: Option<f64>,
    pub active_flow_nodes: Vec<NodeCount>,
}

pub struct Analytics<'a>(pub(crate) &'a Store);

impl Analytics<'_> {
    pub fn operational_summary(
        &self,
        org: OrgId,
        from: i64,
        to: i64,
    ) -> Resultado<OperationalSummary> {
        let org_text = org.to_string();
        self.0.leitura(move |conn| {
            let (open, closed, average, open_conversations) = conn
                .query_row(
                    "SELECT
                        (SELECT COUNT(*) FROM fichas
                          WHERE org_id = ?1 AND status = 'open' AND aberta_em < ?3),
                        (SELECT COUNT(*) FROM fichas
                          WHERE org_id = ?1 AND status = 'closed'
                            AND fechada_em >= ?2 AND fechada_em < ?3),
                        (SELECT AVG(fechada_em - aberta_em) FROM fichas
                          WHERE org_id = ?1 AND status = 'closed'
                            AND fechada_em >= ?2 AND fechada_em < ?3),
                        (SELECT COUNT(*) FROM conversations
                          WHERE org_id = ?1 AND status IN ('open', 'transferred'))",
                    params![org_text, from, to],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?.max(0) as u64,
                            row.get::<_, i64>(1)?.max(0) as u64,
                            row.get::<_, Option<f64>>(2)?,
                            row.get::<_, i64>(3)?.max(0) as u64,
                        ))
                    },
                )
                .map_err(ErroStore::from)?;

            let mut statement = conn
                .prepare(
                    "SELECT e.no_atual, COUNT(*) AS total
                       FROM fluxo_execucoes e
                       JOIN fichas f ON f.org_id = e.org_id AND f.id = e.ficha_id
                      WHERE e.org_id = ?1
                        AND e.status = 'ativa'
                        AND f.status = 'open'
                        AND f.aberta_em < ?2
                      GROUP BY e.no_atual
                      ORDER BY total DESC, e.no_atual ASC",
                )
                .map_err(ErroStore::from)?;
            let nodes = statement
                .query_map(params![org_text, to], |row| {
                    Ok(NodeCount {
                        node_id: row.get(0)?,
                        count: row.get::<_, i64>(1)?.max(0) as u64,
                    })
                })
                .map_err(ErroStore::from)?;
            let mut active_flow_nodes = Vec::new();
            for node in nodes {
                active_flow_nodes.push(node.map_err(ErroStore::from)?);
            }

            Ok(OperationalSummary {
                open_fichas: open,
                closed_fichas: closed,
                open_conversations,
                avg_resolution_ms: average,
                active_flow_nodes,
            })
        })
    }
}
