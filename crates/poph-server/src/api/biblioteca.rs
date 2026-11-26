//! Consulta aos metadados da biblioteca de arquivos de um agente.

use axum::{
    Json,
    extract::{Path, Query, State, rejection::QueryRejection},
    http::StatusCode,
};
use poph_store::{LibraryFile, NewLibraryFile};
use poph_types::{AgentId, FileId, OrgId};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::{ConsultaPagina, ListaJson, proximo_cursor};
use crate::{
    auth::Identidade,
    erro::{ApiError, id_da_url},
    estado::Estado,
};

#[derive(Debug, Deserialize)]
pub struct CaminhoAgente {
    pub org_id: String,
    pub agent_id: String,
}
#[derive(Debug, Deserialize)]
pub struct CaminhoArquivo {
    pub org_id: String,
    pub agent_id: String,
    pub file_id: String,
}

/// Metadados de arquivo, sem caminho interno ou conteúdo.
#[derive(Debug, Serialize, ToSchema)]
pub struct ArquivoJson {
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
impl From<&LibraryFile> for ArquivoJson {
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

#[utoipa::path(get, path = "/admin/v1/orgs/{org_id}/agents/{agent_id}/files", tag = "files", operation_id = "arquivos_listar", security(("bearer" = [])), params(("org_id" = String, Path), ("agent_id" = String, Path), ("limit" = Option<u32>, Query), ("cursor" = Option<String>, Query)), responses((status = 200, description = "Arquivos", body = ListaJson<ArquivoJson>), (status = 403, description = "Org de outro token", body = crate::erro::RespostaErro)))]
pub async fn listar(
    State(estado): State<Estado>,
    identidade: Identidade,
    Path(c): Path<CaminhoAgente>,
    Query(consulta): Query<ConsultaPagina>,
) -> Result<Json<ListaJson<ArquivoJson>>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    identidade.exigir_org(org)?;
    let agente = id_da_url::<AgentId>(&c.agent_id)?;
    estado.store.agents().get(org, agente)?;
    let pagina = consulta.pagina()?;
    let arquivos = estado.store.library().list(org, agente, pagina)?;
    let items: Vec<_> = arquivos.iter().map(ArquivoJson::from).collect();
    let next_cursor = proximo_cursor(&items, pagina.limit, |item| item.id.clone());
    Ok(Json(ListaJson { items, next_cursor }))
}

#[utoipa::path(get, path = "/admin/v1/orgs/{org_id}/agents/{agent_id}/files/{file_id}", tag = "files", operation_id = "arquivos_mostrar", security(("bearer" = [])), params(("org_id" = String, Path), ("agent_id" = String, Path), ("file_id" = String, Path)), responses((status = 200, description = "Arquivo", body = ArquivoJson), (status = 404, description = "Não encontrado", body = crate::erro::RespostaErro)))]
pub async fn mostrar(
    State(estado): State<Estado>,
    identidade: Identidade,
    Path(c): Path<CaminhoArquivo>,
) -> Result<Json<ArquivoJson>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    identidade.exigir_org(org)?;
    let agente = id_da_url::<AgentId>(&c.agent_id)?;
    let id = id_da_url::<FileId>(&c.file_id)?;
    let arquivo = estado.store.library().get(org, agente, id)?;
    Ok(Json(ArquivoJson::from(&arquivo)))
}

/// Conteúdo do arquivo da biblioteca (HR-46b): os bytes do blob, para o Console baixar ou
/// pré-visualizar o que o agente pode enviar ao cliente.
///
/// O escopo é o mesmo das outras rotas do arquivo (org + agente + id). Arquivo de outra org ou de
/// outro agente dá `404`; blob ausente no disco também, com a mesma mensagem genérica — a resposta
/// nunca diz o caminho do blob.
#[utoipa::path(get, path = "/admin/v1/orgs/{org_id}/agents/{agent_id}/files/{file_id}/content", tag = "files", operation_id = "arquivos_conteudo", security(("bearer" = [])), params(("org_id" = String, Path), ("agent_id" = String, Path), ("file_id" = String, Path)), responses((status = 200, description = "Conteúdo do arquivo (bytes com o MIME gravado e o sha256 no cabeçalho `x-content-sha256`)", body = Vec<u8>, content_type = "application/octet-stream"), (status = 403, description = "Org de outro token", body = crate::erro::RespostaErro), (status = 404, description = "Arquivo ou conteúdo não encontrado", body = crate::erro::RespostaErro)))]
pub async fn conteudo(
    State(estado): State<Estado>,
    identidade: Identidade,
    Path(c): Path<CaminhoArquivo>,
) -> Result<axum::response::Response, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    identidade.exigir_org(org)?;
    let agente = id_da_url::<AgentId>(&c.agent_id)?;
    let id = id_da_url::<FileId>(&c.file_id)?;
    let arquivo = estado.store.library().get(org, agente, id)?;
    let pasta = estado.config.blobs();
    let sha256 = arquivo.sha256.clone();
    // A leitura do disco é síncrona e pode levar alguns milissegundos num arquivo de 16 MB.
    let bytes =
        tokio::task::spawn_blocking(move || super::conteudo::ler_blob(&pasta, org, &sha256))
            .await
            .map_err(|_| ApiError::interno())??;
    Ok(super::conteudo::resposta(
        bytes,
        &arquivo.mime,
        &arquivo.name,
        &arquivo.sha256,
    ))
}

/// Exclui um arquivo da biblioteca e registra o autor da mudança.
#[utoipa::path(delete, path = "/admin/v1/orgs/{org_id}/agents/{agent_id}/files/{file_id}", tag = "files", operation_id = "arquivos_excluir", security(("bearer" = [])), params(("org_id" = String, Path), ("agent_id" = String, Path), ("file_id" = String, Path)), responses((status = 200, description = "Arquivo excluído", body = serde_json::Value), (status = 404, description = "Não encontrado", body = crate::erro::RespostaErro)))]
pub async fn excluir(
    State(estado): State<Estado>,
    identidade: Identidade,
    Path(c): Path<CaminhoArquivo>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    identidade.exigir_org(org)?;
    let agente = id_da_url::<AgentId>(&c.agent_id)?;
    let id = id_da_url::<FileId>(&c.file_id)?;
    estado
        .store
        .library()
        .delete_audited(org, agente, id, identidade.ator_id())?;
    Ok(Json(serde_json::json!({"deleted": true})))
}

/// Parâmetros do upload. O corpo da requisição são os bytes do arquivo.
#[derive(Debug, Deserialize)]
pub struct UploadParametros {
    /// Nome do arquivo (usado para mostrar e para conferir a extensão).
    pub filename: String,
    /// Nome mostrado ao cliente. Vazio: o nome do arquivo.
    #[serde(default)]
    pub name: String,
    /// Descrição que o modelo lê para decidir quando enviar.
    #[serde(default)]
    pub description: String,
}

/// Põe um arquivo na biblioteca do agente: mesmo caminho da CLI `poph file upload` (limite de
/// 16 MB, tipo real pelo conteúdo, blob por hash), com auditoria da criação.
#[utoipa::path(post, path = "/admin/v1/orgs/{org_id}/agents/{agent_id}/files", tag = "files", operation_id = "arquivos_criar", security(("bearer" = [])), params(("org_id" = String, Path), ("agent_id" = String, Path), ("filename" = String, Query), ("name" = Option<String>, Query), ("description" = Option<String>, Query)), request_body(content = Vec<u8>, content_type = "application/octet-stream"), responses((status = 201, description = "Arquivo criado", body = ArquivoJson), (status = 400, description = "Arquivo ou parâmetro inválido", body = crate::erro::RespostaErro), (status = 403, description = "Org de outro token", body = crate::erro::RespostaErro)))]
pub async fn criar(
    State(estado): State<Estado>,
    identidade: Identidade,
    Path(c): Path<CaminhoAgente>,
    // A recusa da query é tratada dentro do handler, depois da autorização: assim org de outro
    // token recebe 403 (e não 400) e o erro sai no formato `{code, message, details}`.
    params: Result<Query<UploadParametros>, QueryRejection>,
    request: axum::extract::Request,
) -> Result<(StatusCode, Json<ArquivoJson>), ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    identidade.exigir_org(org)?;
    let agente = id_da_url::<AgentId>(&c.agent_id)?;
    estado.store.agents().get(org, agente)?;
    let Query(params) = params.map_err(|_| {
        ApiError::validacao(
            "filename",
            "informe `?filename=<nome do arquivo>` (e, se quiser, `name` e `description`)",
        )
    })?;

    let config = poph_files::FileConfig::default(estado.config.blobs());
    let limite = usize::try_from(config.byte_limit).unwrap_or(usize::MAX);
    let bytes = axum::body::to_bytes(request.into_body(), limite)
        .await
        .map_err(|_| {
            ApiError::validacao(
                "arquivo",
                "corpo inválido ou maior que o limite de 16 MB por arquivo",
            )
        })?;

    let novo = tokio::task::spawn_blocking(move || {
        let recebido = poph_files::receive(&config, org, &params.filename, &bytes).map_err(
            |erro| match erro {
                poph_files::FileError::Vazio
                | poph_files::FileError::Large { .. }
                | poph_files::FileError::UnsupportedType(_)
                | poph_files::FileError::MisleadingExtension { .. }
                | poph_files::FileError::Reading(_) => {
                    ApiError::validacao("arquivo", erro.to_string())
                }
                _ => ApiError::interno(),
            },
        )?;
        let metadados = recebido.metadados;
        // Mesmo caminho relativo que a CLI grava (`<org>/<sha256>`), para o estado ser o mesmo.
        let blob_path = metadados
            .blob_path
            .strip_prefix(&config.blob_folder)
            .map_err(|_| ApiError::interno())?
            .to_string_lossy()
            .replace('\\', "/");
        Ok::<_, ApiError>(NewLibraryFile {
            agent_id: agente,
            name: if params.name.trim().is_empty() {
                metadados.original_name
            } else {
                params.name
            },
            description: params.description,
            mime: metadados.mime_real,
            size_bytes: metadados.size_bytes,
            sha256: metadados.sha256,
            blob_path,
        })
    })
    .await
    .map_err(|_| ApiError::interno())??;

    let arquivo = estado.store.library().create(org, novo)?;
    super::auditoria::registrar(
        &estado,
        &identidade,
        Some(org),
        "create",
        "file_library",
        arquivo.id.to_string(),
    )?;
    Ok((StatusCode::CREATED, Json(ArquivoJson::from(&arquivo))))
}
