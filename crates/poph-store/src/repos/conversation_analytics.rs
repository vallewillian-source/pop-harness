use std::collections::BTreeMap;

use crate::{
    Store,
    erro::{ErroStore, Resultado},
};
use poph_types::OrgId;
use rusqlite::params;

#[derive(Debug, Clone, Default)]
pub struct ConversationCounts {
    pub total: i64,
    pub open: i64,
    pub transferred: i64,
    pub closed: i64,
}
impl ConversationCounts {
    fn add(&mut self, status: &str) {
        self.total += 1;
        match status {
            "open" => self.open += 1,
            "transferred" => self.transferred += 1,
            "closed" => self.closed += 1,
            _ => {}
        }
    }
}
#[derive(Debug, Clone)]
pub struct ConversationSummary {
    pub counts: ConversationCounts,
    pub avg_messages_per_conversation: Option<f64>,
    pub conversation_to_ficha_rate: Option<f64>,
}
#[derive(Debug, Clone)]
pub struct ConversationDay {
    pub date: String,
    pub counts: ConversationCounts,
}
#[derive(Debug, Clone)]
pub struct ConversationAgent {
    pub agent_id: String,
    pub agent_name: String,
    pub counts: ConversationCounts,
}
#[derive(Debug, Clone)]
pub struct ConversationChannel {
    pub channel: String,
    pub total: i64,
}
#[derive(Debug, Clone)]
pub struct ConversationReport {
    pub summary: ConversationSummary,
    pub by_day: Vec<ConversationDay>,
    pub by_agent: Vec<ConversationAgent>,
    pub by_channel: Vec<ConversationChannel>,
}
pub struct ConversationAnalytics<'a>(pub(crate) &'a Store);
impl ConversationAnalytics<'_> {
    pub fn report(
        &self,
        org: OrgId,
        from: i64,
        to: i64,
        agent: Option<String>,
        channel: Option<String>,
        offset: i64,
    ) -> Resultado<ConversationReport> {
        let org = org.to_string();
        self.0.leitura(move |conn| {
            let tx = conn.unchecked_transaction().map_err(ErroStore::from)?;
            let mut summary = ConversationCounts::default();
            let mut days = BTreeMap::<String, ConversationCounts>::new();
            let mut agents = BTreeMap::<(String, String), ConversationCounts>::new();
            let mut channels = BTreeMap::<String, i64>::new();
            let mut message_total = 0i64;
            let mut with_messages = 0i64;
            let mut with_ficha = 0i64;
            {
                let mut statement = tx.prepare("WITH selected AS MATERIALIZED (SELECT c.id,c.agent_id,c.channel,c.status, COALESCE((SELECT MIN(m.created_at) FROM messages m WHERE m.org_id=c.org_id AND m.conversation_id=c.id),c.created_at) AS first_at FROM conversations c WHERE c.org_id=?1 AND (?4 IS NULL OR c.agent_id=?4) AND (?5 IS NULL OR c.channel=?5)) SELECT s.agent_id,COALESCE(a.name,s.agent_id),s.channel,s.status,date((s.first_at + ?6 * 60000)/1000,'unixepoch'),(SELECT COUNT(*) FROM messages m WHERE m.org_id=?1 AND m.conversation_id=s.id),EXISTS(SELECT 1 FROM fichas f WHERE f.org_id=?1 AND f.conversation_id=s.id) FROM selected s LEFT JOIN agents a ON a.org_id=?1 AND a.id=s.agent_id WHERE s.first_at>=?2 AND s.first_at<?3").map_err(ErroStore::from)?;
                let mut rows = statement.query(params![org,from,to,agent,channel,offset]).map_err(ErroStore::from)?;
                while let Some(row) = rows.next().map_err(ErroStore::from)? {
                    let id: String = row.get(0).map_err(ErroStore::from)?;
                    let name: String = row.get(1).map_err(ErroStore::from)?;
                    let channel: String = row.get(2).map_err(ErroStore::from)?;
                    let status: String = row.get(3).map_err(ErroStore::from)?;
                    let date: String = row.get(4).map_err(ErroStore::from)?;
                    let messages: i64 = row.get(5).map_err(ErroStore::from)?;
                    let ficha: bool = row.get(6).map_err(ErroStore::from)?;
                    summary.add(&status);
                    days.entry(date).or_default().add(&status);
                    agents.entry((id,name)).or_default().add(&status);
                    *channels.entry(channel).or_default() += 1;
                    if messages > 0 { message_total += messages; with_messages += 1; }
                    if ficha { with_ficha += 1; }
                }
            }
            tx.commit().map_err(ErroStore::from)?;
            let avg = (with_messages > 0).then(|| message_total as f64 / with_messages as f64);
            let rate = (summary.total > 0).then(|| with_ficha as f64 / summary.total as f64);
            let by_day = days.into_iter().map(|(date,counts)| ConversationDay {date,counts}).collect();
            let mut by_agent: Vec<_> = agents.into_iter().map(|((agent_id,agent_name),counts)| ConversationAgent {agent_id,agent_name,counts}).collect();
            by_agent.sort_by(|a,b| b.counts.total.cmp(&a.counts.total).then(a.agent_name.cmp(&b.agent_name)).then(a.agent_id.cmp(&b.agent_id)));
            let mut by_channel: Vec<_> = channels.into_iter().map(|(channel,total)| ConversationChannel {channel,total}).collect();
            by_channel.sort_by(|a,b| b.total.cmp(&a.total).then(a.channel.cmp(&b.channel)));
            Ok(ConversationReport {summary: ConversationSummary {counts:summary,avg_messages_per_conversation:avg,conversation_to_ficha_rate:rate},by_day,by_agent,by_channel})
        })
    }
}
