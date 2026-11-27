use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use poph_store::StoreError;
use poph_store::backup::{ImportReport, ImportTarget, ImportUpdates, OrgSnapshot};
use poph_store::backup_document::{self, BackupDocument};
use poph_types::{DomainError, OrgId};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::auth::Identidade;
use crate::erro::{ApiError, id_da_url as parse_path_id};
use crate::estado::Estado;

use super::audit;

pub const LIMIT_IMPORT_BYTES: u64 = 64 * 1024 * 1024;

pub const LIMIT_IMPORT: usize = LIMIT_IMPORT_BYTES as usize;

#[derive(Deserialize)]
pub struct OrgPath {
    org_id: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct ExportParameters {
    #[serde(default)]
    #[serde(rename = "com_conversas")]
    pub include_conversations: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ImportParameters {
    #[serde(default)]
    #[serde(rename = "sobrescrever")]
    pub overwrite: Option<String>,
}

impl ImportParameters {
    fn overwrite(&self) -> Result<bool, ApiError> {
        match self.overwrite.as_deref() {
            None => Ok(false),
            Some(text) if text.eq_ignore_ascii_case("true") => Ok(true),
            Some(text) if text.eq_ignore_ascii_case("false") => Ok(false),
            Some(other) => Err(ApiError::validacao(
                "sobrescrever",
                format!("`{other}` não é um booleano; use true ou false"),
            )),
        }
    }
}

#[doc = " `GET …/backup`: devolve o arquivo do retrato da org."]
#[utoipa::path(get, operation_id="backup_exportar", path="/admin/v1/orgs/{org_id}/backup", tag="backup", security(("bearer" = [])), params(("org_id" = String, Path), ("com_conversas" = Option<bool>, Query, description = "Inclui conversas, mensagens, turnos, as duas filas e os carrinhos (padrão: `false`)")), responses((status = 200, description = "Arquivo do backup da org (JSON no formato `poph-backup-org`, versão 1), como anexo. Segredo de integração sai só com a marca e o nome do campo; caminho de blob não sai", body = Value, content_type = "application/json"), (status = 400, description = "`com_conversas` inválido", body = crate::erro::RespostaErro), (status = 401, description = "Sem token", body = crate::erro::RespostaErro), (status = 403, description = "Org de outro token", body = crate::erro::RespostaErro), (status = 404, description = "Org não encontrada", body = crate::erro::RespostaErro), (status = 409, description = "Arquivo da biblioteca sem conteúdo no disco", body = crate::erro::RespostaErro)))]
pub async fn export(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<OrgPath>,
    Query(q): Query<ExportParameters>,
) -> Result<Response, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let include_conversations = match q.include_conversations.as_deref() {
        None => false,
        Some(text) if text.eq_ignore_ascii_case("true") => true,
        Some(text) if text.eq_ignore_ascii_case("false") => false,
        Some(other) => {
            return Err(ApiError::validacao(
                "com_conversas",
                format!("`{other}` não é um booleano; use true ou false"),
            ));
        }
    };
    e.store.orgs().get(org)?;

    let state = e.clone();
    let (bytes, name) =
        tokio::task::spawn_blocking(move || build_file(&state, org, include_conversations))
            .await
            .map_err(|_| ApiError::interno())??;

    let mut response = (StatusCode::OK, bytes).into_response();
    let header = response.headers_mut();
    header.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    header.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{name}\""))
            .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
    );
    header.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    header.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    Ok(response)
}

fn build_file(
    e: &Estado,
    org: OrgId,
    include_conversations: bool,
) -> Result<(Vec<u8>, String), ApiError> {
    let snapshot = e.store.org_snapshot(org, include_conversations)?;
    let blobs = read_blobs(e, &snapshot, org)?;
    let key = e.chave_de_cifra.clone();
    let name_of_secret = move |blob: &[u8]| -> Option<Vec<String>> {
        let map = key.as_ref()?.decifrar(blob).ok()?;
        Some(map.keys().cloned().collect())
    };
    let document = backup_document::document(
        &snapshot,
        include_conversations,
        e.store.now(),
        &blobs,
        &name_of_secret,
    );
    let bytes = serde_json::to_vec(&document).map_err(|_| {
        ApiError::novo(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "não consegui montar o arquivo de backup",
        )
    })?;
    let slug = snapshot
        .org
        .get("slug")
        .and_then(Value::as_str)
        .unwrap_or("org");
    Ok((bytes, file_name(slug, e.store.now())))
}

fn read_blobs(
    e: &Estado,
    snapshot: &OrgSnapshot,
    org: OrgId,
) -> Result<BTreeMap<String, Vec<u8>>, ApiError> {
    let mut names_by_sha = BTreeMap::new();
    for table in ["file_library", "attachments"] {
        for line in snapshot.tables.get(table).into_iter().flatten() {
            let (Some(sha), Some(name)) = (
                line.get("sha256").and_then(Value::as_str),
                line.get("name").and_then(Value::as_str),
            ) else {
                continue;
            };
            names_by_sha
                .entry(sha.to_owned())
                .or_insert_with(|| name.to_owned());
        }
    }
    let folder = &e.arquivos.blob_folder;
    let limit = e.arquivos.byte_limit;
    let mut blobs = BTreeMap::new();
    for (sha, name) in names_by_sha {
        let bytes = poph_files::blob::read(folder, org, &sha, limit).map_err(|error| {
            tracing::warn!(org = %org, sha = %sha, reason = %error, "exportação sem o conteúdo de um arquivo");
            ApiError::novo(
                StatusCode::CONFLICT,
                "backup_incompleto",
                format!(
                    "o conteúdo do arquivo `{name}` da biblioteca não está no disco: a \
                     exportação não sai pela metade; restaure o arquivo e exporte de novo"
                ),
            )
            .com_detalhes(json!({ "name": name }))
        })?;
        blobs.insert(sha, bytes);
    }
    Ok(blobs)
}

fn file_name(slug: &str, now_ms: i64) -> String {
    let slug: String = slug
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let slug = if slug.trim_matches('-').is_empty() {
        "org".to_owned()
    } else {
        slug
    };
    format!("{slug}-backup-{}.json", stamp(now_ms))
}

fn stamp(now_ms: i64) -> String {
    let second = now_ms.div_euclid(1000);
    let days = second.div_euclid(86_400);
    let in_day = second.rem_euclid(86_400);
    let (year, month, day) = civil_date(days);
    let hour = in_day / 3_600;
    let minute = (in_day % 3_600) / 60;
    format!("{year:04}{month:02}{day:02}-{hour:02}{minute:02}")
}

fn civil_date(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let adjusted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * adjusted_month + 2) / 5 + 1;
    let month = if adjusted_month < 10 {
        adjusted_month + 3
    } else {
        adjusted_month - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    (year, month as u32, day as u32)
}

#[doc = " `POST …/backup/import`: reconstrói a org do caminho a partir do arquivo do corpo."]
#[utoipa::path(post, operation_id="backup_importar", path="/admin/v1/orgs/{org_id}/backup/import", tag="backup", security(("bearer" = [])), params(("org_id" = String, Path), ("sobrescrever" = Option<bool>, Query, description = "Autoriza a importação a atualizar linhas que já existem na org (padrão: `false`, que recusa com `409 importacao_sobrescreve` e nada grava)")), request_body(content = Value, content_type = "application/json", description = "Arquivo gerado por `GET …/backup` (ou `poph backup export`), até 64 MB"), responses((status = 200, description = "Relatório da importação: linhas inseridas/atualizadas por tabela, segredos a digitar de novo e o índice", body = Value), (status = 400, description = "Arquivo corrompido, formato ou versão desconhecidos, ou `sobrescrever` que não é booleano", body = crate::erro::RespostaErro), (status = 401, description = "Sem token", body = crate::erro::RespostaErro), (status = 403, description = "Org de outro token", body = crate::erro::RespostaErro), (status = 404, description = "Org de destino não encontrada", body = crate::erro::RespostaErro), (status = 409, description = "`importacao_sobrescreve` (atualizaria linhas existentes sem `sobrescrever=true`), conflito de chave natural ou arquivo da biblioteca sem conteúdo no disco; nada foi gravado", body = crate::erro::RespostaErro), (status = 413, description = "Arquivo acima do limite", body = crate::erro::RespostaErro)))]
pub async fn import(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<OrgPath>,
    Query(q): Query<ImportParameters>,
    header: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<Value>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let overwrite = q.overwrite()?;

    if let Some(size) = header
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|text| text.parse::<u64>().ok())
        && size > LIMIT_IMPORT_BYTES
    {
        return Err(file_large());
    }
    let bytes = match body {
        Ok(bytes) => bytes,
        Err(rejeicao) => {
            let response = rejeicao.into_response();
            return Err(if response.status() == StatusCode::PAYLOAD_TOO_LARGE {
                file_large()
            } else {
                ApiError::invalido("não consegui ler o corpo da requisição")
            });
        }
    };
    if bytes.len() as u64 > LIMIT_IMPORT_BYTES {
        return Err(file_large());
    }

    e.store.orgs().get(org)?;

    let document = tokio::task::spawn_blocking(move || backup_document::read_document(&bytes))
        .await
        .map_err(|_| ApiError::interno())?
        .map_err(file_refused)?;

    let BackupDocument {
        snapshot,
        blobs,
        com_conversas: include_conversations,
    } = document;
    let state = e.clone();
    let result = tokio::task::spawn_blocking(move || {
        let conflict = state
            .store
            .check_import_conflicts(&snapshot, ImportTarget::Existing(org))
            .map_err(check_error)?;
        if !conflict.is_empty() {
            return Ok(ImportOutcome::Conflict(conflict));
        }
        if !overwrite {
            let update = state
                .store
                .check_import_updates(&snapshot, ImportTarget::Existing(org))
                .map_err(check_error)?;
            if !update.empty() {
                return Ok(ImportOutcome::WouldOverwrite(update));
            }
        }
        write_in_target(&state, org, snapshot, &blobs, overwrite).map(|result| match result {
            Ok(report) => ImportOutcome::Imported(report),
            Err(update) => ImportOutcome::WouldOverwrite(update),
        })
    })
    .await
    .map_err(|_| ApiError::interno())??;
    let report = match result {
        ImportOutcome::Conflict(conflict) => return Err(conflict_of_import(&conflict)),
        ImportOutcome::WouldOverwrite(update) => {
            return Err(overwrites_import(&update));
        }
        ImportOutcome::Imported(report) => report,
    };

    audit::register(
        &e,
        &ident,
        Some(org),
        "backup_import",
        "org",
        org.to_string(),
    )?;

    let (index, warning) = reindex(&e, org).await;
    Ok(Json(json!({
        "org": {"id": report.org.to_string(), "criada": report.created_org},
        "com_conversas": include_conversations,
        "tabelas": table_of_report(&report),
        "total": report.total(),
        "segredos_a_redigitar": report.secrets_to_reenter,
        "indice": index,
        "aviso": warning,
    })))
}

enum ImportOutcome {
    Conflict(Vec<String>),
    WouldOverwrite(ImportUpdates),
    Imported(ImportReport),
}

fn file_large() -> ApiError {
    ApiError::novo(
        StatusCode::PAYLOAD_TOO_LARGE,
        "payload_too_large",
        format!(
            "o arquivo passa do limite de {} MB por pedido; exporte sem as conversas (ou com \
             `com_conversas=false`) ou importe numa instalação com o arquivo menor",
            LIMIT_IMPORT_BYTES / (1024 * 1024)
        ),
    )
}

fn file_refused(error: backup_document::DocumentError) -> ApiError {
    match &error {
        backup_document::DocumentError::Json(_) => ApiError::novo(
            StatusCode::BAD_REQUEST,
            "invalid_backup",
            format!("o corpo não é um backup do POP Harness ({error})"),
        ),
        backup_document::DocumentError::Formato(_) => {
            ApiError::novo(StatusCode::BAD_REQUEST, "invalid_backup", error.to_string())
        }
        backup_document::DocumentError::Versao {
            read: read_version,
            entendida: supported_version,
        } => ApiError::novo(StatusCode::BAD_REQUEST, "invalid_backup", error.to_string())
            .com_detalhes(
                json!({ "version": read_version, "supported_version": supported_version }),
            ),
    }
}

fn conflict_of_import(conflict: &[String]) -> ApiError {
    let shown: Vec<Value> = conflict
        .iter()
        .take(5)
        .cloned()
        .map(Value::String)
        .collect();
    let rest = conflict.len().saturating_sub(shown.len());
    let mut reason = format!(
        "a importação foi recusada antes de gravar: {}",
        conflict
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ")
    );
    if rest > 0 {
        reason.push_str(&format!("; e mais {rest}"));
    }
    reason.push_str(
        ". Nada foi gravado: renomeie um dos dois lados ou importe numa instância sem esses recursos",
    );
    ApiError::novo(StatusCode::CONFLICT, "conflict", reason).com_detalhes(json!({
        "conflicts": shown,
        "conflicts_more": rest,
    }))
}

fn overwrites_import(update: &ImportUpdates) -> ApiError {
    let example: Vec<Value> = update.examples.iter().cloned().map(Value::String).collect();
    let tables: Map<String, Value> = update
        .tables
        .iter()
        .map(|(table, rows)| (table.clone(), Value::from(*rows)))
        .collect();
    ApiError::novo(
        StatusCode::CONFLICT,
        "importacao_sobrescreve",
        format!(
            "a importação atualizaria {} linha(s) que já existem nesta org; nada foi gravado. Repita com `sobrescrever=true` para autorizar",
            update.total
        ),
    )
    .com_detalhes(json!({
        "tabelas": tables,
        "linhas": update.total,
        "exemplos": example,
    }))
}

fn write_in_target(
    e: &Estado,
    org: OrgId,
    snapshot: OrgSnapshot,
    blobs: &BTreeMap<String, Vec<u8>>,
    overwrite: bool,
) -> Result<Result<ImportReport, ImportUpdates>, ApiError> {
    let folder = &e.arquivos.blob_folder;
    for (sha, content) in blobs {
        match poph_files::blob::write(folder, org, content) {
            Ok(stored) if stored.sha256 == *sha => {}
            Ok(_) => {
                return Err(ApiError::novo(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "o conteúdo de um arquivo do backup foi gravado com outro sha256; nada foi importado",
                ));
            }
            Err(error) => {
                tracing::warn!(org = %org, reason = %error, "falha ao gravar o blob da importação");
                return Err(ApiError::novo(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "não consegui gravar o conteúdo de um arquivo do backup",
                ));
            }
        }
    }
    e.store
        .import_protected_snapshot(snapshot, ImportTarget::Existing(org), overwrite)
        .map_err(import_error)
}

fn check_error(error: StoreError) -> ApiError {
    match error {
        StoreError::Domain(DomainError::NotFound { entity, id }) => {
            ApiError::nao_encontrado(entity, &id)
        }
        other => ApiError::from(other),
    }
}

fn import_error(error: StoreError) -> ApiError {
    match error {
        StoreError::Domain(DomainError::Validation {
            field: "backup",
            reason,
        }) => ApiError::novo(StatusCode::BAD_REQUEST, "invalid_backup", reason)
            .com_detalhes(json!({ "field": "backup" })),
        other => ApiError::from(other),
    }
}

fn table_of_report(report: &ImportReport) -> Map<String, Value> {
    report
        .tables
        .iter()
        .map(|(name, stored)| {
            (
                name.clone(),
                json!({"inseridas": stored.inserted, "atualizadas": stored.updated}),
            )
        })
        .collect()
}

async fn reindex(e: &Estado, org: OrgId) -> (Value, Option<String>) {
    use poph_rag::{Embedder, Index, IndexConfig, ModelConfig, OrtEmbedder};

    static SINCRONIZACAO: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = SINCRONIZACAO.lock().await;

    let pending = || {
        json!({
            "base_conhecimento": {"agentes": 0, "trechos": 0},
            "produtos": {"trechos": 0},
        })
    };
    let config = ModelConfig {
        diretorio: e.config.models(),
        ..ModelConfig::default()
    };
    let model = match tokio::task::spawn_blocking(move || OrtEmbedder::open_model(&config, None))
        .await
    {
        Ok(Ok(model)) => model,
        Ok(Err(error)) => {
            return (
                pending(),
                Some(format!(
                    "modelo de embedding indisponível ({error}). O material foi importado, mas o \
                     índice não ficou pronto: rode `poph kb reindex` e `poph product reindex` \
                     quando o modelo estiver disponível"
                )),
            );
        }
        Err(_) => {
            return (
                pending(),
                Some(
                    "não consegui abrir o modelo de embedding. O material foi importado, mas o \
                     índice não ficou pronto: rode `poph kb reindex` e `poph product reindex`"
                        .to_owned(),
                ),
            );
        }
    };
    let embedder: Arc<dyn Embedder> = Arc::new(model);
    let mut notice: Vec<String> = Vec::new();

    let mut agents = 0usize;
    let mut passage = 0usize;
    match Index::open(
        e.store.rag_path(),
        Arc::clone(&embedder),
        IndexConfig::default(),
    ) {
        Ok((index, _)) => {
            let base = poph_agent::KnowledgeBase::new(Arc::clone(&e.store), Arc::new(index));
            match base.synchronise_org(org).await {
                Ok(reports) => {
                    agents = reports.len();
                    passage = reports.iter().map(|(_, report)| report.stored_chunks).sum();
                }
                Err(error) => {
                    tracing::warn!(org = %org, reason = %error, "falha ao reindexar a base da org importada");
                    notice.push(
                        "não consegui reindexar a base de conhecimento; rode `poph kb reindex`"
                            .to_owned(),
                    );
                }
            }
        }
        Err(error) => {
            tracing::warn!(org = %org, reason = %error, "falha ao abrir o índice da base");
            notice.push("não consegui abrir o índice da base; rode `poph kb reindex`".to_owned());
        }
    }

    let mut passage_product = 0usize;
    match Index::open(
        e.store.rag_path(),
        Arc::clone(&embedder),
        IndexConfig::default(),
    ) {
        Ok((index, _)) => {
            let catalog = poph_agent::ProductCatalog::new(Arc::clone(&e.store), Arc::new(index));
            match catalog.synchronise_org(org).await {
                Ok(reports) => {
                    passage_product = reports.iter().map(|(_, report)| report.stored_chunks).sum();
                }
                Err(error) => {
                    tracing::warn!(org = %org, reason = %error, "falha ao reindexar os produtos da org importada");
                    notice.push(
                        "não consegui reindexar os produtos; rode `poph product reindex`"
                            .to_owned(),
                    );
                }
            }
        }
        Err(error) => {
            tracing::warn!(org = %org, reason = %error, "falha ao abrir o índice de produtos");
            notice.push(
                "não consegui abrir o índice de produtos; rode `poph product reindex`".to_owned(),
            );
        }
    }

    (
        json!({
            "base_conhecimento": {"agentes": agents, "trechos": passage},
            "produtos": {"trechos": passage_product},
        }),
        if notice.is_empty() {
            None
        } else {
            Some(notice.join("; "))
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_uses_utc_civil_date() {
        assert_eq!(stamp(0), "19700101-0000");
        assert_eq!(stamp(1_700_000_000_000), "20231114-2213");
        assert_eq!(stamp(1_709_210_096_000), "20240229-1234");
        assert_eq!(civil_date(0), (1970, 1, 1));
    }

    #[test]
    fn file_name_uses_slug_and_stamp() {
        assert_eq!(
            file_name("org-origem", 1_700_000_000_000),
            "org-origem-backup-20231114-2213.json"
        );
        assert_eq!(
            file_name("org origem!", 0),
            "org-origem--backup-19700101-0000.json"
        );
        assert_eq!(file_name("", 0), "org-backup-19700101-0000.json");
    }
}
