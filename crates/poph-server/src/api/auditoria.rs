//! Registro de alterações administrativas sem copiar conteúdo da requisição.
use crate::{auth::Identidade, erro::ApiError, estado::Estado};
use poph_store::NovoRegistroAuditoria;
use poph_types::OrgId;

pub(super) fn registrar(
    estado: &Estado,
    identidade: &Identidade,
    org: Option<OrgId>,
    acao: &str,
    entidade: &str,
    id: String,
) -> Result<(), ApiError> {
    estado.store.auditoria().registrar(
        org,
        NovoRegistroAuditoria {
            ator_tipo: "admin_token".into(),
            ator_id: Some(identidade.ator_id()),
            acao: acao.into(),
            entidade: entidade.into(),
            entidade_id: Some(id),
            detalhe_json: "{}".into(),
        },
    )?;
    Ok(())
}
