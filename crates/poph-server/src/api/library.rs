use axum::{
    Json,
    extract::{Path, Query, State, rejection::QueryRejection},
    http::StatusCode,
};
use poph_store::{LibraryFile, NewLibraryFile};
use poph_types::{AgentId, FileId, OrgId};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::{ListJson, PageQuery, next_cursor};
use crate::{
    auth::Identidade,
    erro::{ApiError, id_da_url as parse_path_id},
    estado::Estado,
};

#[derive(Debug, Deserialize)]
pub struct AgentPath {
    pub org_id: String,
    pub agent_id: String,
}
#[derive(Debug, Deserialize)]
pub struct FilePath {
    pub org_id: String,
    pub agent_id: String,
    pub file_id: String,
}

#[doc = " Metadados de arquivo, sem caminho interno ou conteúdo."]
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ArquivoJson)]
pub struct FileJson {
    pub id: String,
    pub org_id: String,
    pub agent_id: String,
    pub name: String,
    pub description: String,
    pub mime: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub created_at: i64,
}
impl From<&LibraryFile> for FileJson {
    fn from(a: &LibraryFile) -> Self {
        Self {
            id: a.id.to_string(),
            org_id: a.org_id.to_string(),
            agent_id: a.agent_id.to_string(),
            name: a.name.clone(),
            description: a.description.clone(),
            mime: a.mime.clone(),
            size_bytes: a.size_bytes,
            sha256: a.sha256.clone(),
            created_at: a.created_at,
        }
    }
}

#[utoipa::path(get, path = "/admin/v1/orgs/{org_id}/agents/{agent_id}/files", tag = "files", operation_id = "arquivos_listar", security(("bearer" = [])), params(("org_id" = String, Path), ("agent_id" = String, Path), ("limit" = Option<u32>, Query), ("cursor" = Option<String>, Query)), responses((status = 200, description = "Arquivos", body = ListJson<FileJson>), (status = 403, description = "Org de outro token", body = crate::erro::RespostaErro)))]
pub async fn list(
    State(state): State<Estado>,
    identity: Identidade,
    Path(c): Path<AgentPath>,
    Query(query): Query<PageQuery>,
) -> Result<Json<ListJson<FileJson>>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    identity.exigir_org(org)?;
    let agent_id = parse_path_id::<AgentId>(&c.agent_id)?;
    state.store.agents().get(org, agent_id)?;
    let page = query.page()?;
    let file = state.store.library().list(org, agent_id, page)?;
    let items: Vec<_> = file.iter().map(FileJson::from).collect();
    let next_cursor = next_cursor(&items, page.limit, |item| item.id.clone());
    Ok(Json(ListJson { items, next_cursor }))
}

#[utoipa::path(get, path = "/admin/v1/orgs/{org_id}/agents/{agent_id}/files/{file_id}", tag = "files", operation_id = "arquivos_mostrar", security(("bearer" = [])), params(("org_id" = String, Path), ("agent_id" = String, Path), ("file_id" = String, Path)), responses((status = 200, description = "Arquivo", body = FileJson), (status = 404, description = "Não encontrado", body = crate::erro::RespostaErro)))]
pub async fn show(
    State(state): State<Estado>,
    identity: Identidade,
    Path(c): Path<FilePath>,
) -> Result<Json<FileJson>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    identity.exigir_org(org)?;
    let agent_id = parse_path_id::<AgentId>(&c.agent_id)?;
    let id = parse_path_id::<FileId>(&c.file_id)?;
    let file = state.store.library().get(org, agent_id, id)?;
    Ok(Json(FileJson::from(&file)))
}

#[doc = " Conteúdo do arquivo da biblioteca (HR-46b): os bytes do blob, para o Console baixar ou"]
#[doc = " pré-visualizar o que o agente pode enviar ao cliente."]
#[doc = ""]
#[doc = " O escopo é o mesmo das outras rotas do arquivo (org + agente + id). Arquivo de outra org ou de"]
#[doc = " outro agente dá `404`; blob ausente no disco também, com a mesma mensagem genérica — a resposta"]
#[doc = " nunca diz o caminho do blob."]
#[utoipa::path(get, path = "/admin/v1/orgs/{org_id}/agents/{agent_id}/files/{file_id}/content", tag = "files", operation_id = "arquivos_conteudo", security(("bearer" = [])), params(("org_id" = String, Path), ("agent_id" = String, Path), ("file_id" = String, Path)), responses((status = 200, description = "Conteúdo do arquivo (bytes com o MIME gravado e o sha256 no cabeçalho `x-content-sha256`)", body = Vec<u8>, content_type = "application/octet-stream"), (status = 403, description = "Org de outro token", body = crate::erro::RespostaErro), (status = 404, description = "Arquivo ou conteúdo não encontrado", body = crate::erro::RespostaErro)))]
pub async fn content(
    State(state): State<Estado>,
    identity: Identidade,
    Path(c): Path<FilePath>,
) -> Result<axum::response::Response, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    identity.exigir_org(org)?;
    let agent_id = parse_path_id::<AgentId>(&c.agent_id)?;
    let id = parse_path_id::<FileId>(&c.file_id)?;
    let file = state.store.library().get(org, agent_id, id)?;
    let folder = state.config.blobs();
    let sha256 = file.sha256.clone();
    let bytes =
        tokio::task::spawn_blocking(move || super::content::read_blob(&folder, org, &sha256))
            .await
            .map_err(|_| ApiError::interno())??;
    Ok(super::content::response(
        bytes,
        &file.mime,
        &file.name,
        &file.sha256,
    ))
}

#[doc = " Exclui um arquivo da biblioteca e registra o autor da mudança."]
#[utoipa::path(delete, path = "/admin/v1/orgs/{org_id}/agents/{agent_id}/files/{file_id}", tag = "files", operation_id = "arquivos_excluir", security(("bearer" = [])), params(("org_id" = String, Path), ("agent_id" = String, Path), ("file_id" = String, Path)), responses((status = 200, description = "Arquivo excluído", body = serde_json::Value), (status = 404, description = "Não encontrado", body = crate::erro::RespostaErro)))]
pub async fn delete(
    State(state): State<Estado>,
    identity: Identidade,
    Path(c): Path<FilePath>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    identity.exigir_org(org)?;
    let agent_id = parse_path_id::<AgentId>(&c.agent_id)?;
    let id = parse_path_id::<FileId>(&c.file_id)?;
    state
        .store
        .library()
        .delete_audited(org, agent_id, id, identity.ator_id())?;
    Ok(Json(serde_json::json!({"deleted": true})))
}

#[derive(Debug, Deserialize)]
pub struct UploadParameters {
    pub filename: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
}

#[doc = " Põe um arquivo na biblioteca do agente: mesmo caminho da CLI `poph file upload` (limite de"]
#[doc = " 16 MB, tipo real pelo conteúdo, blob por hash), com auditoria da criação."]
#[utoipa::path(post, path = "/admin/v1/orgs/{org_id}/agents/{agent_id}/files", tag = "files", operation_id = "arquivos_criar", security(("bearer" = [])), params(("org_id" = String, Path), ("agent_id" = String, Path), ("filename" = String, Query), ("name" = Option<String>, Query), ("description" = Option<String>, Query)), request_body(content = Vec<u8>, content_type = "application/octet-stream"), responses((status = 201, description = "Arquivo criado", body = FileJson), (status = 400, description = "Arquivo ou parâmetro inválido", body = crate::erro::RespostaErro), (status = 403, description = "Org de outro token", body = crate::erro::RespostaErro)))]
pub async fn create(
    State(state): State<Estado>,
    identity: Identidade,
    Path(c): Path<AgentPath>,
    params: Result<Query<UploadParameters>, QueryRejection>,
    request: axum::extract::Request,
) -> Result<(StatusCode, Json<FileJson>), ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    identity.exigir_org(org)?;
    let agent_id = parse_path_id::<AgentId>(&c.agent_id)?;
    state.store.agents().get(org, agent_id)?;
    let Query(params) = params.map_err(|_| {
        ApiError::validacao(
            "filename",
            "informe `?filename=<nome do arquivo>` (e, se quiser, `name` e `description`)",
        )
    })?;

    let config = poph_files::FileConfig::default(state.config.blobs());
    let limit = usize::try_from(config.byte_limit).unwrap_or(usize::MAX);
    let bytes = axum::body::to_bytes(request.into_body(), limit)
        .await
        .map_err(|_| {
            ApiError::validacao(
                "arquivo",
                "corpo inválido ou maior que o limite de 16 MB por arquivo",
            )
        })?;

    let new_file = tokio::task::spawn_blocking(move || {
        let receive = poph_files::receive(&config, org, &params.filename, &bytes).map_err(
            |error| match error {
                poph_files::FileError::Vazio
                | poph_files::FileError::Large { .. }
                | poph_files::FileError::UnsupportedType(_)
                | poph_files::FileError::MisleadingExtension { .. }
                | poph_files::FileError::Reading(_) => {
                    ApiError::validacao("arquivo", error.to_string())
                }
                _ => ApiError::interno(),
            },
        )?;
        let metadata = receive.metadados;
        let blob_path = metadata
            .blob_path
            .strip_prefix(&config.blob_folder)
            .map_err(|_| ApiError::interno())?
            .to_string_lossy()
            .replace('\\', "/");
        Ok::<_, ApiError>(NewLibraryFile {
            agent_id,
            name: if params.name.trim().is_empty() {
                metadata.original_name
            } else {
                params.name
            },
            description: params.description,
            mime: metadata.mime_real,
            size_bytes: metadata.size_bytes,
            sha256: metadata.sha256,
            blob_path,
        })
    })
    .await
    .map_err(|_| ApiError::interno())??;

    let file = state.store.library().create(org, new_file)?;
    super::audit::register(
        &state,
        &identity,
        Some(org),
        "create",
        "file_library",
        file.id.to_string(),
    )?;
    Ok((StatusCode::CREATED, Json(FileJson::from(&file))))
}
