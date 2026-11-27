use crate::{auth::Identity, error::ApiError, estado::Estado};
use poph_store::NewAuditEntry;
use poph_types::OrgId;

pub(super) fn register(
    state: &Estado,
    identity: &Identity,
    org: Option<OrgId>,
    acao: &str,
    entity: &str,
    id: String,
) -> Result<(), ApiError> {
    state.store.audit().register(
        org,
        NewAuditEntry {
            ator_tipo: "admin_token".into(),
            actor_id: Some(identity.actor_id()),
            acao: acao.into(),
            entity: entity.into(),
            entity_id: Some(id),
            detail_json: "{}".into(),
        },
    )?;
    Ok(())
}
