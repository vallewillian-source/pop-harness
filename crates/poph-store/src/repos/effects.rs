use poph_types::{EffectId, EffectState, OrgId};
use rusqlite::{OptionalExtension, params};

use super::executions::{COLUMNS_EFFECT, effect_row};
use crate::Store;
use crate::error::{Resultado, StoreError};
use crate::model::FlowEffect;

pub struct Effects<'a>(pub(crate) &'a Store);

impl Effects<'_> {
    pub fn orgs_with_pending(&self) -> Resultado<Vec<OrgId>> {
        let now = self.0.now();
        self.0.read(move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT DISTINCT org_id FROM fluxo_efeitos
                      WHERE estado IN ('pendente', 'enviando')
                        AND (proxima_tentativa IS NULL OR proxima_tentativa <= ?1)
                      ORDER BY org_id",
                )
                .map_err(StoreError::from)?;
            let rows = statement
                .query_map(params![now], |row| row.get::<_, String>(0))
                .map_err(StoreError::from)?;
            let mut orgs = Vec::new();
            for row in rows {
                let texto = row.map_err(StoreError::from)?;
                orgs.push(texto.parse().map_err(StoreError::Domain)?);
            }
            Ok(orgs)
        })
    }

    pub fn list_pending(&self, org: OrgId, limit: u32) -> Resultado<Vec<FlowEffect>> {
        let now = self.0.now();
        let text_org = org.to_string();
        let limit = i64::from(limit.clamp(1, 1000));
        self.0.read(move |conn| {
            let mut statement = conn
                .prepare(&format!(
                    "SELECT {COLUMNS_EFFECT} FROM fluxo_efeitos
                      WHERE org_id = ?1 AND estado IN ('pendente', 'enviando')
                        AND (proxima_tentativa IS NULL OR proxima_tentativa <= ?2)
                      ORDER BY created_at, id LIMIT ?3"
                ))
                .map_err(StoreError::from)?;
            let rows = statement
                .query_map(params![text_org, now, limit], effect_row)
                .map_err(StoreError::from)?;
            let mut effects = Vec::new();
            for row in rows {
                effects.push(row.map_err(StoreError::from)?);
            }
            Ok(effects)
        })
    }

    pub fn get(&self, org: OrgId, id: EffectId) -> Resultado<FlowEffect> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        let located = self.0.read(move |conn| {
            conn.query_row(
                &format!(
                    "SELECT {COLUMNS_EFFECT} FROM fluxo_efeitos WHERE org_id = ?1 AND id = ?2"
                ),
                params![text_org, text_id],
                effect_row,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(located, "efeito", &id.to_string())
    }

    pub fn start_send(&self, org: OrgId, id: EffectId) -> Resultado<u32> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        self.0.write(move |conn| {
            let attempts = conn
                .query_row(
                    "UPDATE fluxo_efeitos
                        SET estado = 'enviando', tentativas = tentativas + 1, ultimo_erro = NULL
                      WHERE org_id = ?1 AND id = ?2 AND estado IN ('pendente', 'enviando')
                      RETURNING tentativas",
                    params![text_org, text_id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(StoreError::from)?
                .ok_or_else(|| {
                    StoreError::Internal(format!(
                        "efeito `{text_id}` não estava aguardando entrega"
                    ))
                })?;
            Ok(attempts.max(0) as u32)
        })
    }

    pub fn confirm_send(&self, org: OrgId, id: EffectId) -> Resultado<()> {
        let now = self.0.now();
        let text_org = org.to_string();
        let text_id = id.to_string();
        self.0.write(move |conn| {
            conn.execute(
                "UPDATE fluxo_efeitos
                    SET estado = 'enviado', enviado_em = ?3, proxima_tentativa = NULL
                  WHERE org_id = ?1 AND id = ?2",
                params![text_org, text_id, now],
            )
            .map(|_| ())
            .map_err(StoreError::from)
        })
    }

    pub fn fail_send(
        &self,
        org: OrgId,
        id: EffectId,
        motivo: &str,
        definitivo: bool,
        proxima_tentativa: Option<i64>,
    ) -> Resultado<()> {
        let state = if definitivo {
            EffectState::Failed
        } else {
            EffectState::Pending
        };
        let next = if definitivo { None } else { proxima_tentativa };
        let text_org = org.to_string();
        let text_id = id.to_string();
        let motivo = motivo.to_owned();
        self.0.write(move |conn| {
            conn.execute(
                "UPDATE fluxo_efeitos
                    SET estado = ?3, ultimo_erro = ?4, proxima_tentativa = ?5, enviado_em = NULL
                  WHERE org_id = ?1 AND id = ?2",
                params![text_org, text_id, state.as_text(), motivo, next],
            )
            .map(|_| ())
            .map_err(StoreError::from)
        })
    }

    pub fn adiar(&self, org: OrgId, id: EffectId, proxima_tentativa: i64) -> Resultado<()> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        self.0.write(move |conn| {
            conn.execute(
                "UPDATE fluxo_efeitos
                    SET proxima_tentativa = ?3
                  WHERE org_id = ?1 AND id = ?2 AND estado = 'pendente'",
                params![text_org, text_id, proxima_tentativa],
            )
            .map(|_| ())
            .map_err(StoreError::from)
        })
    }
}
