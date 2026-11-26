//! Registro de alterações administrativas sem copiar conteúdo da requisição.
use crate::{auth::Identidade, erro::ApiError, estado::Estado};
use poph_store::NewAuditEntry;
use poph_types::OrgId;

pub(super) fn registrar(
    estado: &Estado,
    identidade: &Identidade,
    org: Option<OrgId>,
    acao: &str,
    entidade: &str,
    id: String,
) -> Result<(), ApiError> {
    estado.store.audit().register(
        org,
        NewAuditEntry {
            ator_tipo: "admin_token".into(),
            actor_id: Some(identidade.ator_id()),
            acao: acao.into(),
            entity: entidade.into(),
            entity_id: Some(id),
            detail_json: "{}".into(),
        },
    )?;
    Ok(())
}
