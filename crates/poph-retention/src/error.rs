#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RetentionError {
    #[error("retenção da org {org} inválida: {motivo}. Nada foi apagado nessa org.")]
    InvalidConfig { org: String, motivo: String },
    #[error("não existe org com o id {0}. Nada foi apagado.")]
    OrgNotFound(String),
    #[error("falha no banco: {0}")]
    Database(String),
    #[error("falha nos arquivos em {path}: {motivo}")]
    Files { path: String, motivo: String },
}
