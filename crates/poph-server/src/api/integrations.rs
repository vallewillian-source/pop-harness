use std::collections::BTreeMap;

use super::{ListJson, PageQuery, if_match_version, next_cursor};
use crate::{
    auth::Identity,
    cipher::CipherKey,
    error::{ApiError, id_from_url as parse_path_id},
    estado::Estado,
    manifestos::{FalhaManifesto, validar_objeto},
};
use axum::{
    Json,
    extract::{
        Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, StatusCode},
};
use poph_store::{AdminIntegration, IntegrationChange};
use poph_types::{AgentId, IntegrationId, Manifest, OrgId};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
pub struct PathParams {
    org_id: String,
    agent_id: String,
}
#[derive(Deserialize)]
pub struct ItemPath {
    org_id: String,
    agent_id: String,
    integration_id: String,
}
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Dados {
    kind: Option<String>,
    config_json: Option<Value>,
    secrets: Option<BTreeMap<String, String>>,
    status: Option<String>,
    connection_state: Option<String>,
}
fn validate_status(status: Option<String>) -> Result<Option<String>, ApiError> {
    if let Some(s) = &status {
        super::request_status(s)?;
    }
    Ok(status)
}
fn ids(org: &str, agent_id: &str, ident: &Identity) -> Result<(OrgId, AgentId), ApiError> {
    let org = parse_path_id::<OrgId>(org)?;
    ident.require_org(org)?;
    Ok((org, parse_path_id::<AgentId>(agent_id)?))
}

fn value(v: AdminIntegration, key: Option<&CipherKey>) -> Value {
    let filled = match (&v.encrypted_secrets, key) {
        (None, _) => Some(Vec::<String>::new()),
        (Some(_), None) => None,
        (Some(blob), Some(key)) => key
            .decifrar(blob)
            .ok()
            .map(|secrets| secrets.into_keys().collect()),
    };
    json!({
        "id": v.id,
        "kind": v.kind,
        "status": v.status,
        "connection_state": v.connection_state,
        "version": v.version,
        "tem_segredo": v.has_secret,
        "configuracao": v.configuracao,
        "segredos_preenchidos": filled,
    })
}

fn empty_object() -> Value {
    Value::Object(Default::default())
}

fn sent_configuration(value: &Option<Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Object(map)) => !map.is_empty(),
        Some(_) => true,
    }
}

fn sent_secrets(value: &Option<BTreeMap<String, String>>) -> bool {
    value.as_ref().is_some_and(|map| !map.is_empty())
}

fn check(
    field: &'static str,
    values: Option<&Value>,
    schema: &poph_types::JsonSchema,
) -> Result<(), ApiError> {
    let Some(values) = values else {
        return Ok(());
    };
    validar_objeto(values, &schema.0, field)
        .map_err(|failure| ApiError::validation(failure.campo, failure.motivo))
}

async fn kind_manifest(e: &Estado, kind: &str) -> Result<Option<Manifest>, ApiError> {
    match e.manifestos.manifesto(kind).await {
        Ok(manifesto) => Ok(Some(manifesto)),
        Err(FalhaManifesto::NaoDeclarado) => Ok(None),
        Err(failure @ FalhaManifesto::Indisponivel { .. }) => Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "manifesto_indisponivel",
            failure.mensagem(),
        )
        .with_details(json!({ "kind": kind }))),
    }
}

fn without_manifest() -> ApiError {
    ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "manifest_indisponivel",
        "configuração de integração exige o manifesto do plugin, disponível na entrega de \
         hospedagem de plugins",
    )
}

struct Prepared {
    configuration: Option<Value>,
    secrets: Option<Option<Vec<u8>>>,
}

fn prepare(
    e: &Estado,
    manifesto: &Manifest,
    configuration: Option<&Value>,
    secrets: Option<BTreeMap<String, String>>,
) -> Result<Prepared, ApiError> {
    let configuration = match configuration {
        None => None,
        Some(Value::Null) => Some(empty_object()),
        Some(value) => {
            check("config_json", Some(value), &manifesto.configuration)?;
            Some(value.clone())
        }
    };
    let Some(secrets) = secrets else {
        return Ok(Prepared {
            configuration,
            secrets: None,
        });
    };
    let secrets: BTreeMap<String, String> = secrets
        .into_iter()
        .filter(|(_, value)| !value.is_empty())
        .collect();
    let as_value = Value::Object(
        secrets
            .iter()
            .map(|(name, value)| (name.clone(), Value::String(value.clone())))
            .collect(),
    );
    check("secrets", Some(&as_value), &manifesto.secrets)?;
    if secrets.is_empty() {
        return Ok(Prepared {
            configuration,
            secrets: Some(None),
        });
    }
    let key = e.chave_de_cifra.as_deref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "cifra_indisponivel",
            "o servidor está sem a chave de cifra (`POPH_CHAVE_CIFRA`): segredo de integração não \
             é gravado em claro",
        )
    })?;
    let blob = key.cifrar(&secrets).map_err(|failure| {
        tracing::error!(%failure, "falha ao cifrar segredo de integração");
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "cifra_falhou",
            "não consegui cifrar o segredo da integração",
        )
    })?;
    Ok(Prepared {
        configuration,
        secrets: Some(Some(blob)),
    })
}

fn merge_secrets(
    e: &Estado,
    new: BTreeMap<String, String>,
    previous: Option<&[u8]>,
) -> Result<BTreeMap<String, String>, ApiError> {
    let mut merged = match previous {
        None => BTreeMap::new(),
        Some(blob) => {
            let key = e.chave_de_cifra.as_deref().ok_or_else(|| {
                ApiError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "cifra_indisponivel",
                    "o servidor está sem a chave de cifra (`POPH_CHAVE_CIFRA`): não consigo ler os \
                     segredos já gravados para mesclar os novos",
                )
            })?;
            key.decifrar(blob).map_err(|failure| {
                tracing::error!(%failure, "segredo de integração ilegível com a chave atual");
                ApiError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "cifra_indisponivel",
                    format!("os segredos gravados não abrem com a chave atual: {failure}"),
                )
            })?
        }
    };
    for (name, value) in new {
        if value.is_empty() {
            merged.remove(&name);
        } else {
            merged.insert(name, value);
        }
    }
    Ok(merged)
}

#[utoipa::path(operation_id = "listar", get,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations",tag="integrations",params(("org_id"=String,Path),("agent_id"=String,Path),("limit"=Option<u32>,Query),("cursor"=Option<String>,Query)),security(("bearer"=[])),responses((status=200,description="Integrações",body=ListJson<Value>)))]
pub async fn list(
    State(e): State<Estado>,
    ident: Identity,
    Path(c): Path<PathParams>,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Result<Json<ListJson<Value>>, ApiError> {
    let (org, agent_id) = ids(&c.org_id, &c.agent_id, &ident)?;
    let Query(q) = query.map_err(|_| ApiError::invalid("parâmetros de paginação inválidos"))?;
    let page = q.page()?;
    let items: Vec<_> = e
        .store
        .list_integrations(org, agent_id, page)?
        .into_iter()
        .map(|v| value(v, e.chave_de_cifra.as_deref()))
        .collect();
    let next_cursor = next_cursor(&items, page.limit, |v| {
        v["id"].as_str().unwrap_or_default().to_owned()
    });
    Ok(Json(ListJson { items, next_cursor }))
}

#[utoipa::path(operation_id = "mostrar", get,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}",tag="integrations",params(("org_id"=String,Path),("agent_id"=String,Path),("integration_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Integração",body=Value)))]
pub async fn show(
    State(e): State<Estado>,
    ident: Identity,
    Path(c): Path<ItemPath>,
) -> Result<Json<Value>, ApiError> {
    let (org, agent_id) = ids(&c.org_id, &c.agent_id, &ident)?;
    let id = parse_path_id::<IntegrationId>(&c.integration_id)?;
    Ok(Json(value(
        e.store.get_integration(org, agent_id, id)?,
        e.chave_de_cifra.as_deref(),
    )))
}

#[utoipa::path(operation_id = "criar", post,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations",tag="integrations",params(("org_id"=String,Path),("agent_id"=String,Path)),security(("bearer"=[])),request_body=Dados,responses((status=201,description="Integração criada",body=Value)))]
pub async fn create(
    State(e): State<Estado>,
    ident: Identity,
    Path(c): Path<PathParams>,
    body: Result<Json<Dados>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let (org, agent_id) = ids(&c.org_id, &c.agent_id, &ident)?;
    let Json(d) = body.map_err(|_| ApiError::invalid("corpo JSON de integração inválido"))?;
    if d.connection_state.is_some() {
        return Err(ApiError::validation(
            "connection_state",
            "não permitido na criação",
        ));
    }
    let kind = d
        .kind
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| ApiError::validation("kind", "obrigatório"))?;
    let status = validate_status(d.status)?.unwrap_or_else(|| "active".into());
    let send = sent_configuration(&d.config_json) || sent_secrets(&d.secrets);
    let (configuration, secrets) = match kind_manifest(&e, &kind).await? {
        Some(manifesto) => {
            let without_configuration = empty_object();
            let prepared = prepare(
                &e,
                &manifesto,
                Some(d.config_json.as_ref().unwrap_or(&without_configuration)),
                Some(d.secrets.clone().unwrap_or_default()),
            )?;
            (
                prepared.configuration.unwrap_or_else(empty_object),
                prepared.secrets.flatten(),
            )
        }
        None if send => return Err(without_manifest()),
        None => (empty_object(), None),
    };
    let v = e.store.create_integration_with_config(
        org,
        agent_id,
        kind,
        status,
        configuration,
        secrets,
    )?;
    super::audit::register(&e, &ident, Some(org), "create", "integration", v.id.clone())?;
    Ok((
        StatusCode::CREATED,
        Json(value(v, e.chave_de_cifra.as_deref())),
    ))
}

#[utoipa::path(operation_id = "atualizar", patch,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}",tag="integrations",params(("org_id"=String,Path),("agent_id"=String,Path),("integration_id"=String,Path),("If-Match"=String,Header)),security(("bearer"=[])),request_body=Dados,responses((status=200,description="Integração alterada",body=Value),(status=412,description="Conflito",body=crate::error::ErrorResponse)))]
pub async fn update(
    State(e): State<Estado>,
    ident: Identity,
    Path(c): Path<ItemPath>,
    h: HeaderMap,
    body: Result<Json<Dados>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let (org, agent_id) = ids(&c.org_id, &c.agent_id, &ident)?;
    let id = parse_path_id::<IntegrationId>(&c.integration_id)?;
    let version = if_match_version(&h, "integração")?;
    let Json(d) = body.map_err(|_| ApiError::invalid("corpo JSON de integração inválido"))?;
    if d.kind.is_some() {
        return Err(ApiError::validation("kind", "não pode ser alterado"));
    }
    let touches_configuration = sent_configuration(&d.config_json) || sent_secrets(&d.secrets);
    if d.status.is_none() && d.connection_state.is_none() && !touches_configuration {
        return Err(ApiError::invalid("nada a alterar"));
    }
    let status = validate_status(d.status)?;
    let prepared = if touches_configuration {
        let current = e.store.get_integration(org, agent_id, id)?;
        match kind_manifest(&e, &current.kind).await? {
            Some(manifesto) => {
                let secrets = match d.secrets.clone() {
                    None => None,
                    Some(new) => Some(merge_secrets(
                        &e,
                        new,
                        current.encrypted_secrets.as_deref(),
                    )?),
                };
                Some(prepare(&e, &manifesto, d.config_json.as_ref(), secrets)?)
            }
            None => return Err(without_manifest()),
        }
    } else {
        None
    };
    let (configuration, secrets) = match prepared {
        None => (None, None),
        Some(prepared) => (prepared.configuration, prepared.secrets),
    };
    let v = e.store.update_integration(
        org,
        agent_id,
        id,
        IntegrationChange {
            version,
            status: status.as_deref(),
            state: d.connection_state.as_deref(),
            configuracao: configuration.as_ref(),
            segredos: secrets.as_ref().map(|blob| blob.as_deref()),
        },
    )?;
    super::audit::register(
        &e,
        &ident,
        Some(org),
        "update",
        "integration",
        id.to_string(),
    )?;
    Ok(Json(value(v, e.chave_de_cifra.as_deref())))
}

#[utoipa::path(operation_id = "excluir", delete,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}",tag="integrations",params(("org_id"=String,Path),("agent_id"=String,Path),("integration_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Integração excluída",body=Value)))]
pub async fn delete(
    State(e): State<Estado>,
    ident: Identity,
    Path(c): Path<ItemPath>,
) -> Result<Json<Value>, ApiError> {
    let (org, agent_id) = ids(&c.org_id, &c.agent_id, &ident)?;
    let id = parse_path_id::<IntegrationId>(&c.integration_id)?;
    e.store.delete_integration(org, agent_id, id)?;
    super::audit::register(
        &e,
        &ident,
        Some(org),
        "delete",
        "integration",
        id.to_string(),
    )?;
    Ok(Json(json!({"deleted":true})))
}
