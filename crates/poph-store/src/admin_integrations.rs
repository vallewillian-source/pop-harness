use crate::{
    Store,
    error::{Resultado, StoreError},
    model::Page,
};
use poph_types::{AgentId, DomainError, IntegrationId, OrgId};
use rusqlite::{OptionalExtension, Row, params};
use serde_json::Value;

#[derive(Debug, Default)]
pub struct IntegrationChange<'a> {
    pub version: u64,
    pub status: Option<&'a str>,
    pub state: Option<&'a str>,
    pub configuracao: Option<&'a Value>,
    pub segredos: Option<Option<&'a [u8]>>,
}

pub struct AdminIntegration {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub connection_state: String,
    pub version: u64,
    pub has_secret: bool,
    pub configuracao: Value,
    pub encrypted_secrets: Option<Vec<u8>>,
}

impl std::fmt::Debug for AdminIntegration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdminIntegration")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("status", &self.status)
            .field("connection_state", &self.connection_state)
            .field("version", &self.version)
            .field("tem_segredo", &self.has_secret)
            .field("configuracao", &self.configuracao)
            .field(
                "segredos_cifrados",
                &self.encrypted_secrets.as_ref().map(|_| "…"),
            )
            .finish()
    }
}

fn row(r: &Row<'_>) -> rusqlite::Result<AdminIntegration> {
    let configuracao: String = r.get(6)?;
    Ok(AdminIntegration {
        id: r.get(0)?,
        kind: r.get(1)?,
        status: r.get(2)?,
        connection_state: r.get(3)?,
        version: r.get::<_, i64>(4)? as u64,
        has_secret: r.get(5)?,
        configuracao: serde_json::from_str(&configuracao)
            .unwrap_or_else(|_| Value::Object(Default::default())),
        encrypted_secrets: r.get(7)?,
    })
}
const COLUMNS: &str = "id, kind, status, connection_state, version, secrets_enc IS NOT NULL, config_json, secrets_enc";
impl Store {
    pub fn list_integrations(
        &self,
        org: OrgId,
        agent: AgentId,
        page: Page,
    ) -> Resultado<Vec<AdminIntegration>> {
        self.agents().get(org, agent)?;
        self.read(|conn| {
            let mut q = conn.prepare(&format!("SELECT {COLUMNS} FROM integrations WHERE org_id=?1 AND agent_id=?2 AND (?3 IS NULL OR id>?3) ORDER BY id LIMIT ?4"))?;
            let rows = q.query_map(params![org.to_string(),agent.to_string(),page.after.map(|x|x.to_string()),page.limit],row)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }
    pub fn get_integration(
        &self,
        org: OrgId,
        agent: AgentId,
        id: IntegrationId,
    ) -> Resultado<AdminIntegration> {
        let item = self.read(|conn| Ok(conn.query_row(&format!("SELECT {COLUMNS} FROM integrations WHERE org_id=?1 AND agent_id=?2 AND id=?3"),params![org.to_string(),agent.to_string(),id.to_string()],row).optional()?))?;
        crate::readers::require(item, "integração", &id.to_string())
    }
    pub fn create_integration(
        &self,
        org: OrgId,
        agent: AgentId,
        kind: String,
        status: String,
    ) -> Resultado<AdminIntegration> {
        self.create_integration_with_config(
            org,
            agent,
            kind,
            status,
            Value::Object(Default::default()),
            None,
        )
    }

    pub fn create_integration_with_config(
        &self,
        org: OrgId,
        agent: AgentId,
        kind: String,
        status: String,
        configuracao: Value,
        segredos: Option<Vec<u8>>,
    ) -> Resultado<AdminIntegration> {
        self.agents().get(org, agent)?;
        let id = IntegrationId::generate();
        let now = self.now();
        let configuracao = configuracao.to_string();
        self.write(move |conn| { conn.execute("INSERT INTO integrations (id,org_id,agent_id,kind,status,config_json,secrets_enc,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?8)",params![id.to_string(),org.to_string(),agent.to_string(),kind,status,configuracao,segredos,now])?; Ok(()) })?;
        self.get_integration(org, agent, id)
    }
    pub fn update_integration(
        &self,
        org: OrgId,
        agent: AgentId,
        id: IntegrationId,
        change: IntegrationChange<'_>,
    ) -> Resultado<AdminIntegration> {
        let now = self.now();
        let IntegrationChange {
            version,
            status,
            state,
            configuracao,
            segredos,
        } = change;
        let status = status.map(str::to_owned);
        let state = state.map(str::to_owned);
        let configuracao = configuracao.map(|value| value.to_string());
        let (clear_secrets, segredos) = match segredos {
            None => (false, None),
            Some(None) => (true, None),
            Some(Some(blob)) => (false, Some(blob.to_vec())),
        };
        self.write(move |conn| {
            let n=conn.execute(
                "UPDATE integrations SET status=COALESCE(?4,status), connection_state=COALESCE(?5,connection_state), config_json=COALESCE(?6,config_json), secrets_enc=CASE WHEN ?7 THEN NULL ELSE COALESCE(?8,secrets_enc) END, version=version+1, updated_at=?9 WHERE org_id=?1 AND agent_id=?2 AND id=?3 AND version=?10",
                params![org.to_string(),agent.to_string(),id.to_string(),status,state,configuracao,clear_secrets,segredos,now,version as i64],
            )?;
            if n==0 {
                let current: Option<i64>=conn.query_row("SELECT version FROM integrations WHERE org_id=?1 AND agent_id=?2 AND id=?3",params![org.to_string(),agent.to_string(),id.to_string()],|r|r.get(0)).optional()?;
                return Err(StoreError::Domain(match current {Some(current)=>DomainError::VersionConflict{entity:"integração",id:id.to_string(),expected:version,current:current as u64},None=>DomainError::NotFound{entity:"integração",id:id.to_string()}}));
            }
            Ok(())
        })?;
        self.get_integration(org, agent, id)
    }
    pub fn delete_integration(
        &self,
        org: OrgId,
        agent: AgentId,
        id: IntegrationId,
    ) -> Resultado<()> {
        self.write(move |conn| {
            let n = conn.execute(
                "DELETE FROM integrations WHERE org_id=?1 AND agent_id=?2 AND id=?3",
                params![org.to_string(), agent.to_string(), id.to_string()],
            )?;
            if n == 0 {
                return Err(StoreError::Domain(DomainError::NotFound {
                    entity: "integração",
                    id: id.to_string(),
                }));
            }
            Ok(())
        })
    }
}

impl Store {
    pub fn channel_integration(
        &self,
        org: OrgId,
        integration: IntegrationId,
    ) -> Resultado<Option<crate::model::ChannelIntegration>> {
        let org_text = org.to_string();
        let integration_text = integration.to_string();
        let found = self.read(move |conn| {
            conn.query_row(
                "SELECT agent_id, kind, status FROM integrations WHERE org_id = ?1 AND id = ?2",
                params![org_text, integration_text],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        match found {
            None => Ok(None),
            Some((agent, kind, status)) => Ok(Some(crate::model::ChannelIntegration {
                id: integration,
                agent_id: agent.parse::<AgentId>()?,
                kind,
                status,
            })),
        }
    }

    pub fn set_integration_connection_state(
        &self,
        org: OrgId,
        integration: IntegrationId,
        state: &str,
    ) -> Resultado<bool> {
        let now = self.now();
        let org_text = org.to_string();
        let integration_text = integration.to_string();
        let state = state.to_owned();
        self.write(move |conn| {
            let changed = conn
                .execute(
                    "UPDATE integrations SET connection_state = ?3, updated_at = ?4
                      WHERE org_id = ?1 AND id = ?2",
                    params![org_text, integration_text, state, now],
                )
                .map_err(StoreError::from)?;
            Ok(changed > 0)
        })
    }
}
