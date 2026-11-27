use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use poph_store::{NewOrg, Org, OrgChange, Page, Status};
use poph_types::OrgId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::pagination::{ListJson, PageQuery, next_cursor};
use super::{if_match_version, request_status, status_text};
use crate::auth::Identity;
use crate::error::{ApiError, id_from_url as parse_path_id};
use crate::state::ServerState;

#[doc = " Org na resposta da API."]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OrgJson {
    #[doc = " Id (UUID v7)."]
    pub id: String,
    #[doc = " Nome de exibição."]
    pub name: String,
    #[doc = " Identificador na URL."]
    pub slug: String,
    #[doc = " `active`, `inactive` ou `archived`."]
    pub status: String,
    #[doc = " Configuração da org (retenção, limites, padrões)."]
    #[schema(value_type = Object)]
    pub config: Value,
    #[doc = " Versão para `If-Match`."]
    pub version: u64,
    #[doc = " Criação (ms desde a época Unix)."]
    pub created_at: i64,
    #[doc = " Última alteração (ms)."]
    pub updated_at: i64,
    #[doc = " Quantos agentes **ativos** (`status = active`) a org tem hoje (CF-03a). Vem sempre, nas duas"]
    #[doc = " formas da listagem (com e sem `?cluster=true`) e na leitura de uma org só; `0` quando não há"]
    #[doc = " agente ativo. É contado por uma consulta agregada por página, nunca uma por linha."]
    #[doc = ""]
    #[doc = " `serde(default)` só vale na **leitura**: é o que faz a listagem do cluster continuar"]
    #[doc = " funcionando quando uma instância vizinha ainda não tem o campo (versão anterior)."]
    #[serde(default)]
    pub agents_active_count: u32,
    #[doc = " Instância dona da org (HR-50b-2). Só aparece com `?cluster=true`; sem o parâmetro, a"]
    #[doc = " resposta é a de sempre, sem este campo."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instancia: Option<String>,
}

impl OrgJson {
    fn de(org: &Org) -> Self {
        let config = serde_json::from_str(&org.config_json)
            .unwrap_or_else(|_| Value::Object(Default::default()));
        Self {
            id: org.id.to_string(),
            name: org.name.clone(),
            slug: org.slug.clone(),
            status: status_text(org.status).to_owned(),
            config,
            version: org.version,
            created_at: org.created_at,
            updated_at: org.updated_at,
            agents_active_count: 0,
            instancia: None,
        }
    }

    fn on_instance(mut self, instancia: &str) -> Self {
        self.instancia = Some(instancia.to_owned());
        self
    }

    fn with_active_agents(mut self, active: u32) -> Self {
        self.agents_active_count = active;
        self
    }
}

fn items_with_active_agents(
    state: &ServerState,
    orgs: &[Org],
    instancia: Option<&str>,
) -> Result<Vec<OrgJson>, ApiError> {
    let ids: Vec<OrgId> = orgs.iter().map(|org| org.id).collect();
    let count = state.store.agents().count_active_by_org(&ids)?;
    Ok(orgs
        .iter()
        .map(|org| {
            let item =
                OrgJson::de(org).with_active_agents(count.get(&org.id).copied().unwrap_or(0));
            match instancia {
                Some(instancia) => item.on_instance(instancia),
                None => item,
            }
        })
        .collect())
}

#[doc = " Lista de orgs. Sem `?cluster=true` é a mesma resposta de sempre (`items` + `next_cursor`);"]
#[doc = " com o parâmetro, `instancias` soma o estado de cada instância do mapa e cada item traz a sua"]
#[doc = " instância em `instancia`."]
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = OrgsListJson)]
pub struct OrgsListJson {
    #[doc = " Os registros desta página (ou todas as orgs, com `?cluster=true`)."]
    pub items: Vec<OrgJson>,
    #[doc = " Cursor para a próxima página; `null` na última (e sempre `null` com `?cluster=true`)."]
    pub next_cursor: Option<String>,
    #[doc = " Estado das instâncias do mapa, quando `?cluster=true`. Ausente sem o parâmetro."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "instancias")]
    pub instance: Option<Vec<InstanceStatusJson>>,
}

impl OrgsListJson {
    fn local(list: ListJson<OrgJson>) -> Self {
        Self {
            items: list.items,
            next_cursor: list.next_cursor,
            instance: None,
        }
    }
}

#[doc = " Estado de uma instância na listagem com `?cluster=true`."]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = InstanciaStatusJson)]
pub struct InstanceStatusJson {
    #[doc = " Id da instância no mapa."]
    pub id: String,
    #[doc = " Endereço base declarado no mapa."]
    pub url: String,
    #[doc = " `local` (esta instância), `ok` (respondeu) ou `inacessivel`."]
    pub situacao: String,
    #[doc = " Quantas orgs desta instância vieram nesta resposta."]
    pub orgs: u32,
    #[doc = " Frase de operação quando a instância não respondeu (nada de segredo nem caminho)."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mensagem: Option<String>,
}

#[doc = " Corpo de criação de org."]
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[schema(as = CriarOrg)]
pub struct CreateOrg {
    #[doc = " Nome de exibição (1 a 120 caracteres)."]
    pub name: String,
    #[doc = " Identificador na URL; sem ele, é derivado do nome."]
    pub slug: Option<String>,
    #[doc = " Configuração inicial (objeto JSON)."]
    #[schema(value_type = Option<Object>)]
    pub config: Option<Value>,
}

#[doc = " Corpo de alteração de org (só o que vier muda)."]
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[schema(as = AtualizarOrg)]
pub struct UpdateOrg {
    #[doc = " Novo nome."]
    pub name: Option<String>,
    #[doc = " Nova situação."]
    pub status: Option<String>,
    #[doc = " Nova configuração (objeto JSON)."]
    #[schema(value_type = Option<Object>)]
    pub config: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OrgPath {
    pub org_id: String,
}

#[doc = " Parâmetros de exclusão."]
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[schema(as = OrgDeletion)]
pub struct ExclusionOrg {
    #[doc = " Confirma a exclusão de uma org que tem conteúdo."]
    pub force: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct OrgsQuery {
    pub limit: Option<u32>,
    pub cursor: Option<String>,
    pub cluster: Option<bool>,
}

impl OrgsQuery {
    fn page(&self) -> Result<Page, ApiError> {
        PageQuery {
            limit: self.limit,
            cursor: self.cursor.clone(),
        }
        .page()
    }
}

const DEADLINE_OF_LIST_REMOTE: std::time::Duration = std::time::Duration::from_secs(2);

const PAGE_OF_LIST_REMOTE: u32 = 20;

#[doc = " `GET /admin/v1/orgs`."]
#[utoipa::path(
    get,
    path = "/admin/v1/orgs",
    tag = "orgs",
    operation_id = "orgs_listar",
    security(("bearer" = [])),
    params(
        ("limit" = Option<u32>, Query, description = "Registros por página (1 a 200, padrão 50); ignorado com `cluster=true`"),
        ("cursor" = Option<String>, Query, description = "Último id da página anterior; ignorado com `cluster=true`"),
        ("cluster" = Option<bool>, Query, description = "Com `true`, soma as orgs das outras instâncias do mapa (token global) e devolve o estado de cada instância em `instancias`; uma instância fora do ar aparece como `inacessivel` sem derrubar a lista")
    ),
    responses(
        (status = 200, description = "Página de orgs. Cada item traz `agents_active_count` (agentes com `status = active`, `0` quando não há nenhum), nas duas formas da listagem. Com `cluster=true`, `items` traz todas as orgs do cluster (cada uma com `instancia`) e `instancias` traz o estado de cada instância; sem o parâmetro é a página de sempre", body = ListJson<OrgJson>),
        (status = 401, description = "Token ausente ou inválido", body = crate::error::ErrorResponse)
    )
)]
pub async fn list(
    State(state): State<ServerState>,
    identity: Identity,
    Query(query): Query<OrgsQuery>,
    header: HeaderMap,
) -> Result<Json<OrgsListJson>, ApiError> {
    if let Some(own_org) = identity.org {
        let org = state.store.orgs().get(own_org)?;
        return Ok(Json(OrgsListJson::local(ListJson {
            items: items_with_active_agents(&state, std::slice::from_ref(&org), None)?,
            next_cursor: None,
        })));
    }
    if query.cluster.unwrap_or(false) {
        return list_in_cluster(&state, &header).await.map(Json);
    }
    let page = query.page()?;
    let orgs = state.store.orgs().list(page)?;
    let items = items_with_active_agents(&state, &orgs, None)?;
    let next_cursor = next_cursor(&items, page.limit, |item| item.id.clone());
    Ok(Json(OrgsListJson::local(ListJson { items, next_cursor })))
}

async fn list_in_cluster(
    state: &ServerState,
    header: &HeaderMap,
) -> Result<OrgsListJson, ApiError> {
    let cluster = state.cluster();
    let local = cluster.config.instance_id.trim().to_owned();
    let mut all = Vec::new();
    let mut after = None;
    for _ in 0..PAGE_OF_LIST_REMOTE {
        let page = state.store.orgs().list(Page { after, limit: 200 })?;
        after = page.last().map(|org| org.id.into());
        let last = page.len() < 200 || after.is_none();
        all.extend(page);
        if last {
            break;
        }
    }
    let mut items: Vec<OrgJson> = items_with_active_agents(state, &all, Some(&local))?;
    let mut instance = vec![InstanceStatusJson {
        id: local.clone(),
        url: cluster
            .config
            .instances
            .iter()
            .find(|instancia| instancia.id.trim() == local)
            .map(|instancia| instancia.url.trim().to_owned())
            .unwrap_or_default(),
        situacao: "local".to_owned(),
        orgs: u32::try_from(all.len()).unwrap_or(u32::MAX),
        mensagem: None,
    }];

    let authorization = header
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let client = reqwest::Client::builder()
        .connect_timeout(DEADLINE_OF_LIST_REMOTE)
        .timeout(DEADLINE_OF_LIST_REMOTE)
        .build()
        .map_err(|_: reqwest::Error| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "não consegui montar o cliente HTTP da listagem entre instâncias",
            )
        })?;

    let query = cluster
        .config
        .instances
        .iter()
        .filter(|instancia| instancia.id.trim() != local)
        .map(|instancia| {
            let client = client.clone();
            let authorization = authorization.clone();
            let id = instancia.id.trim().to_owned();
            let url = instancia.url.trim().to_owned();
            async move {
                let result = instance_orgs(&client, &url, authorization.as_deref()).await;
                (id, url, result)
            }
        })
        .collect::<Vec<_>>();

    for (id, url, result) in futures::future::join_all(query).await {
        match result {
            Ok(remote_instances) => {
                let quantity = u32::try_from(remote_instances.len()).unwrap_or(u32::MAX);
                items.extend(
                    remote_instances
                        .into_iter()
                        .map(|org| org.on_instance(&id))
                        .collect::<Vec<_>>(),
                );
                instance.push(InstanceStatusJson {
                    id,
                    url,
                    situacao: "ok".to_owned(),
                    orgs: quantity,
                    mensagem: None,
                });
            }
            Err(reason) => instance.push(InstanceStatusJson {
                id,
                url,
                situacao: "inacessivel".to_owned(),
                orgs: 0,
                mensagem: Some(reason),
            }),
        }
    }

    items.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(OrgsListJson {
        items,
        next_cursor: None,
        instance: Some(instance),
    })
}

async fn instance_orgs(
    client: &reqwest::Client,
    url: &str,
    authorization: Option<&str>,
) -> Result<Vec<OrgJson>, String> {
    let base = url.trim().trim_end_matches('/');
    let mut orgs: Vec<OrgJson> = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..PAGE_OF_LIST_REMOTE {
        let mut address = format!("{base}/admin/v1/orgs?limit=200");
        if let Some(cursor) = cursor.as_deref() {
            address.push_str("&cursor=");
            address.push_str(cursor);
        }
        let mut request = client.get(&address);
        if let Some(authorization) = authorization {
            request = request.header(axum::http::header::AUTHORIZATION, authorization);
        }
        let response = request
            .send()
            .await
            .map_err(|error| format!("não respondeu: {error}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "o /admin/v1/orgs respondeu {} para este token",
                response.status().as_u16()
            ));
        }
        let text = response
            .text()
            .await
            .map_err(|error| format!("não consegui ler a resposta: {error}"))?;
        let body: Value = serde_json::from_str(&text)
            .map_err(|_| "a resposta do /admin/v1/orgs não é JSON".to_owned())?;
        let page: Vec<OrgJson> = serde_json::from_value(
            body.get("items")
                .cloned()
                .unwrap_or_else(|| Value::Array(Vec::new())),
        )
        .map_err(|_| {
            "a resposta do /admin/v1/orgs não trouxe orgs no formato esperado".to_owned()
        })?;
        orgs.extend(page);
        cursor = body
            .get("next_cursor")
            .and_then(|value| value.as_str())
            .map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    Ok(orgs)
}

#[doc = " `POST /admin/v1/orgs` (só token global)."]
#[utoipa::path(
    post,
    path = "/admin/v1/orgs",
    tag = "orgs",
    operation_id = "orgs_criar",
    security(("bearer" = [])),
    request_body = CreateOrg,
    responses(
        (status = 201, description = "Org criada", body = OrgJson),
        (status = 400, description = "Campo inválido", body = crate::error::ErrorResponse),
        (status = 403, description = "Token de org não cria org", body = crate::error::ErrorResponse),
        (status = 409, description = "Slug já existe", body = crate::error::ErrorResponse)
    )
)]
pub async fn create(
    State(state): State<ServerState>,
    identity: Identity,
    Json(body): Json<CreateOrg>,
) -> Result<(StatusCode, Json<OrgJson>), ApiError> {
    identity.require_global()?;
    let name = validate_name(&body.name)?;
    let slug = match body.slug.as_deref().map(str::trim) {
        Some(text) if !text.is_empty() => text.to_owned(),
        _ => derive_slug(&name),
    };
    if !valid_slug(&slug) {
        return Err(ApiError::validation(
            "slug",
            format!("`{slug}`: use de 3 a 64 caracteres, só a-z, 0-9 e `-`"),
        ));
    }
    if state.store.orgs().by_slug(&slug).is_ok() {
        return Err(ApiError::conflict(format!(
            "já existe uma org com o slug `{slug}`"
        )));
    }
    let config_json = match &body.config {
        None | Some(Value::Null) => "{}".to_owned(),
        Some(Value::Object(_)) => serde_json::to_string(body.config.as_ref().unwrap())
            .map_err(|_| ApiError::validation("config", "não consegui serializar o objeto"))?,
        Some(_) => {
            return Err(ApiError::validation(
                "config",
                "deve ser um objeto JSON (ex.: {\"retencao\":{\"dias\":30}})",
            ));
        }
    };
    let org = state.store.orgs().create(NewOrg {
        name,
        slug,
        config_json,
    })?;
    super::audit::register(
        &state,
        &identity,
        Some(org.id),
        "create",
        "org",
        org.id.to_string(),
    )?;
    Ok((StatusCode::CREATED, Json(OrgJson::de(&org))))
}

#[doc = " `GET /admin/v1/orgs/{org_id}`."]
#[utoipa::path(
    get,
    path = "/admin/v1/orgs/{org_id}",
    tag = "orgs",
    operation_id = "orgs_mostrar",
    security(("bearer" = [])),
    params(("org_id" = String, Path, description = "Id da org (UUID v7)")),
    responses(
        (status = 200, description = "Org", body = OrgJson),
        (status = 403, description = "Org de outro token", body = crate::error::ErrorResponse),
        (status = 404, description = "Org não encontrada", body = crate::error::ErrorResponse)
    )
)]
pub async fn show(
    State(state): State<ServerState>,
    identity: Identity,
    Path(path): Path<OrgPath>,
) -> Result<Json<OrgJson>, ApiError> {
    let id = parse_path_id::<OrgId>(&path.org_id)?;
    identity.require_org(id)?;
    let org = state.store.orgs().get(id)?;
    let item = items_with_active_agents(&state, std::slice::from_ref(&org), None)?
        .pop()
        .unwrap_or_else(|| OrgJson::de(&org));
    Ok(Json(item))
}

#[doc = " `PATCH /admin/v1/orgs/{org_id}` (exige `If-Match`)."]
#[utoipa::path(
    patch,
    path = "/admin/v1/orgs/{org_id}",
    tag = "orgs",
    operation_id = "orgs_atualizar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path, description = "Id da org"),
        ("If-Match" = String, Header, description = "Versão lida antes")
    ),
    request_body = UpdateOrg,
    responses(
        (status = 200, description = "Org alterada", body = OrgJson),
        (status = 412, description = "Versão desatualizada", body = crate::error::ErrorResponse),
        (status = 428, description = "Faltou If-Match", body = crate::error::ErrorResponse)
    )
)]
pub async fn update(
    State(state): State<ServerState>,
    identity: Identity,
    Path(path): Path<OrgPath>,
    headers: HeaderMap,
    Json(body): Json<UpdateOrg>,
) -> Result<Json<OrgJson>, ApiError> {
    let id = parse_path_id::<OrgId>(&path.org_id)?;
    identity.require_org(id)?;
    let expected = if_match_version(&headers, "org")?;
    let status = match &body.status {
        Some(text) => Some(request_status(text)?),
        None => None,
    };
    let config_json = match &body.config {
        None => None,
        Some(Value::Object(_)) => Some(
            serde_json::to_string(body.config.as_ref().unwrap())
                .map_err(|_| ApiError::validation("config", "não consegui serializar o objeto"))?,
        ),
        Some(_) => {
            return Err(ApiError::validation("config", "deve ser um objeto JSON"));
        }
    };
    let change = OrgChange {
        name: body.name.as_deref().map(validate_name).transpose()?,
        status,
        config_json,
    };
    let org = state.store.orgs().update(id, expected, change)?;
    super::audit::register(&state, &identity, Some(id), "update", "org", id.to_string())?;
    let item = items_with_active_agents(&state, std::slice::from_ref(&org), None)?
        .pop()
        .unwrap_or_else(|| OrgJson::de(&org));
    Ok(Json(item))
}

#[doc = " `DELETE /admin/v1/orgs/{org_id}?force=true` (só token global)."]
#[utoipa::path(
    delete,
    path = "/admin/v1/orgs/{org_id}",
    tag = "orgs",
    operation_id = "orgs_excluir",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path, description = "Id da org"),
        ("force" = Option<bool>, Query, description = "Confirma a exclusão de org com conteúdo")
    ),
    responses(
        (status = 204, description = "Org excluída"),
        (status = 409, description = "Org tem agentes e faltou force", body = crate::error::ErrorResponse)
    )
)]
pub async fn delete(
    State(state): State<ServerState>,
    identity: Identity,
    Path(path): Path<OrgPath>,
    Query(deletion): Query<ExclusionOrg>,
) -> Result<StatusCode, ApiError> {
    let id = parse_path_id::<OrgId>(&path.org_id)?;
    identity.require_global()?;
    if deletion.force.unwrap_or(false) {
        state.store.orgs().delete_forced(id)?;
    } else {
        state.store.orgs().delete(id)?;
    }
    super::audit::register(
        &state,
        &identity,
        None,
        if deletion.force.unwrap_or(false) {
            "delete_force"
        } else {
            "delete"
        },
        "org",
        id.to_string(),
    )?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) fn validate_name(text: &str) -> Result<String, ApiError> {
    let clean = text.trim();
    let size = clean.chars().count();
    if !(1..=120).contains(&size) {
        return Err(ApiError::validation(
            "name",
            "deve ter de 1 a 120 caracteres",
        ));
    }
    Ok(clean.to_owned())
}

pub(crate) fn valid_slug(slug: &str) -> bool {
    let size = slug.chars().count();
    (3..=64).contains(&size)
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !slug.starts_with('-')
        && !slug.ends_with('-')
}

fn derive_slug(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    let mut last_trace = false;
    for character in name.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            last_trace = false;
        } else if !last_trace && !slug.is_empty() {
            slug.push('-');
            last_trace = true;
        }
    }
    let slug = slug.trim_end_matches('-').to_owned();
    if slug.len() < 3 {
        format!("org-{slug}")
    } else {
        slug
    }
}

#[allow(dead_code)]
pub(crate) fn test_page() -> Page {
    Page::default()
}

#[allow(dead_code)]
pub(crate) fn active_status() -> Status {
    Status::Active
}
