use crate::{auth::Identidade, erro::ApiError, estado::Estado};
use poph_store::NewAuditEntry;
use poph_types::OrgId;

pub(super) fn register(
    state: &Estado,
    identity: &Identidade,
    org: Option<OrgId>,
    acao: &str,
    entity: &str,
    id: String,
) -> Result<(), ApiError> {
    state.store.audit().register(
        org,
        NewAuditEntry {
            ator_tipo: "admin_token".into(),
            actor_id: Some(identity.ator_id()),
            acao: acao.into(),
            entity: entity.into(),
            entity_id: Some(id),
            detail_json: "{}".into(),
        },
    )?;
    Ok(())
}
