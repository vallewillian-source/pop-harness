use crate::{
    Store,
    erro::{ErroStore, Resultado},
};
use poph_types::OrgId;
use rusqlite::params;

#[derive(Debug, Clone)]
pub struct AgentUsage {
    pub agent_id: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub turns: i64,
}
#[derive(Debug, Clone)]
pub struct DayUsage {
    pub date: String,
    pub output_tokens: i64,
    pub turns: i64,
}
#[derive(Debug, Clone)]
pub struct TokenUsage {
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cost_micros: i64,
    pub turns: i64,
    pub by_agent: Vec<AgentUsage>,
    pub by_day: Vec<DayUsage>,
}
pub struct Usage<'a>(pub(crate) &'a Store);
impl Usage<'_> {
    pub fn tokens(&self, org: OrgId, from: i64, to: i64) -> Resultado<TokenUsage> {
        let org = org.to_string();
        self.0.leitura(move |conn| {
            let tx = conn.unchecked_transaction().map_err(ErroStore::from)?;
            let (input, output, cost, turns) = tx.query_row(
                "SELECT COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0), COALESCE(SUM(cost_micros),0), COUNT(*) FROM turns WHERE org_id=?1 AND started_at>=?2 AND started_at<?3",
                params![org,from,to], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))
            ).map_err(ErroStore::from)?;
            let by_agent = {
                let mut stmt = tx.prepare("SELECT agent_id, SUM(input_tokens), SUM(output_tokens), COUNT(*) FROM turns WHERE org_id=?1 AND started_at>=?2 AND started_at<?3 GROUP BY agent_id ORDER BY agent_id IS NULL, agent_id").map_err(ErroStore::from)?;
                let rows = stmt.query_map(params![org,from,to], |r| Ok(AgentUsage { agent_id:r.get(0)?, input_tokens:r.get(1)?, output_tokens:r.get(2)?, turns:r.get(3)? })).map_err(ErroStore::from)?;
                rows.collect::<Result<Vec<_>,_>>().map_err(ErroStore::from)?
            };
            let by_day = {
                let mut stmt = tx.prepare("SELECT date(started_at / 1000.0, 'unixepoch'), SUM(output_tokens), COUNT(*) FROM turns WHERE org_id=?1 AND started_at>=?2 AND started_at<?3 GROUP BY 1 ORDER BY 1").map_err(ErroStore::from)?;
                let rows = stmt.query_map(params![org,from,to], |r| Ok(DayUsage { date:r.get(0)?, output_tokens:r.get(1)?, turns:r.get(2)? })).map_err(ErroStore::from)?;
                rows.collect::<Result<Vec<_>,_>>().map_err(ErroStore::from)?
            };
            tx.commit().map_err(ErroStore::from)?;
            Ok(TokenUsage { total_input_tokens:input,total_output_tokens:output,total_cost_micros:cost,turns,by_agent,by_day })
        })
    }
}
