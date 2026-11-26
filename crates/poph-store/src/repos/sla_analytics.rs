use crate::{
    Store,
    erro::{ErroStore, Resultado},
};
use poph_types::OrgId;
use rusqlite::params;

#[derive(Debug, Clone)]
pub struct SlaItem {
    pub ficha_id: String,
    pub conversation_id: Option<String>,
    pub channel: String,
    pub peer: Option<String>,
    pub client_name: Option<String>,
    pub current_step_key: Option<String>,
    pub age_ms: i64,
    pub idle_ms: i64,
}
#[derive(Debug, Clone)]
pub struct SlaReport {
    pub ages: Vec<i64>,
    pub stale_items: Vec<SlaItem>,
}
pub struct SlaAnalytics<'a>(pub(crate) &'a Store);
impl SlaAnalytics<'_> {
    pub fn report(&self, org: OrgId, to: i64, now: i64, limit: usize) -> Resultado<SlaReport> {
        let org = org.to_string();
        self.0.leitura(move |conn| {
            let tx = conn.unchecked_transaction().map_err(ErroStore::from)?;
            let mut items = Vec::new();
            {
                let mut stmt = tx.prepare("SELECT f.id,f.conversation_id,COALESCE(c.channel,f.canal),f.peer_externo,c.client_name,(SELECT e.no_atual FROM fluxo_execucoes e WHERE e.org_id=f.org_id AND e.ficha_id=f.id AND e.status='ativa'),f.aberta_em,COALESCE((SELECT MAX(v.created_at) FROM ficha_eventos v WHERE v.org_id=f.org_id AND v.ficha_id=f.id),f.aberta_em) FROM fichas f LEFT JOIN conversations c ON c.org_id=f.org_id AND c.id=f.conversation_id WHERE f.org_id=?1 AND f.status='open' AND f.aberta_em<?2").map_err(ErroStore::from)?;
                let mut rows = stmt.query(params![org,to]).map_err(ErroStore::from)?;
                while let Some(row) = rows.next().map_err(ErroStore::from)? {
                    let opened: i64 = row.get(6).map_err(ErroStore::from)?;
                    let last: i64 = row.get(7).map_err(ErroStore::from)?;
                    items.push(SlaItem { ficha_id: row.get(0).map_err(ErroStore::from)?, conversation_id: row.get(1).map_err(ErroStore::from)?, channel: row.get(2).map_err(ErroStore::from)?, peer: row.get(3).map_err(ErroStore::from)?, client_name: row.get(4).map_err(ErroStore::from)?, current_step_key: row.get(5).map_err(ErroStore::from)?, age_ms: now.saturating_sub(opened), idle_ms: now.saturating_sub(last) });
                }
            }
            tx.commit().map_err(ErroStore::from)?;
            let ages = items.iter().map(|i| i.age_ms).collect();
            items.sort_by(|a,b| b.idle_ms.cmp(&a.idle_ms).then(b.age_ms.cmp(&a.age_ms)).then(b.ficha_id.cmp(&a.ficha_id)));
            items.truncate(limit);
            Ok(SlaReport { ages, stale_items: items })
        })
    }
}
