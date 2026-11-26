//! Administração de agentes por organização.
use super::orgs::CaminhoOrg;
use super::{ConsultaPagina, ListaJson, proximo_cursor, status_pedido, versao_do_if_match};
use crate::{
    auth::Identidade,
    erro::{ApiError, id_da_url},
    estado::Estado,
};
use axum::{
    Json,
    extract::{Path, Query, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
};
use poph_store::{AgentChange, Agente, NewAgent};
use poph_types::{AgentId, DepartmentId, OrgId, ProductCategoryId};
use serde::Deserialize;
use serde_json::{Value, json};
use utoipa::ToSchema;

#[derive(Deserialize)]
pub struct CaminhoAgente {
    org_id: String,
    agent_id: String,
}

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DadosAgente {
    name: Option<String>,
    slug: Option<String>,
    description: Option<String>,
    #[serde(rename = "type")]
    tipo: Option<String>,
    category: Option<String>,
    department: Option<String>,
    handoff_department: Option<String>,
    handoff: Option<bool>,
    provider: Option<String>,
    model: Option<String>,
    temperature: Option<f64>,
    max_output_tokens: Option<i64>,
    max_context_tokens: Option<u32>,
    grouping_window_ms: Option<u32>,
    max_tool_rounds: Option<u32>,
    max_tool_calls: Option<u32>,
    rag_top_k: Option<u32>,
    rag_min_score: Option<f64>,
    jev_json: Option<String>,
    status: Option<String>,
    product_category_ids: Option<Vec<String>>,
}

fn json_agente(a: &Agente) -> Value {
    json!({"id":a.id.to_string(),"org_id":a.org_id.to_string(),"name":a.name,"slug":a.slug,
        "description":a.description,"status":a.status.as_text(),"abre_turno":a.can_open_turn(),
        "agent_type":a.agent_type,"category":a.category,"default_department_id":a.default_department.map(|v|v.to_string()),
        "human_handoff_enabled":a.handoff_active,"human_handoff_department_id":a.handoff_department.map(|v|v.to_string()),
        "provider":a.provedor,"model":a.modelo,"temperature":a.temperatura,"max_output_tokens":a.max_tokens_saida,
        "max_context_messages":a.max_context_messages,"max_context_tokens":a.max_context_tokens,
        "grouping_window_ms":a.grouping_window_ms,"max_tool_rounds":a.max_tool_rounds,
        "max_tool_calls_per_message":a.max_tools_per_message,"rag_top_k":a.rag_top_k,
        "rag_min_score":a.rag_cutoff,"jev":serde_json::from_str::<Value>(&a.jev_json).unwrap_or(json!({})),
        "version":a.version,"created_at":a.created_at,"updated_at":a.updated_at})
}
fn slug(nome: &str) -> String {
    let mut s = String::new();
    let mut traco = false;
    for c in nome.chars().flat_map(char::to_lowercase).map(|c| match c {
        'á' | 'à' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ç' => 'c',
        'ñ' => 'n',
        x => x,
    }) {
        if c.is_ascii_alphanumeric() {
            s.push(c);
            traco = false;
        } else if !traco && !s.is_empty() {
            s.push('-');
            traco = true;
        }
    }
    while s.ends_with('-') {
        s.pop();
    }
    s.truncate(64);
    while s.ends_with('-') {
        s.pop();
    }
    s
}
fn dep(s: Option<String>) -> Result<Option<DepartmentId>, ApiError> {
    s.map(|v| id_da_url::<DepartmentId>(&v)).transpose()
}

#[utoipa::path(get,path="/admin/v1/orgs/{org_id}/agents",tag="agents",params(("org_id"=String,Path),("limit"=Option<u32>,Query),("cursor"=Option<String>,Query)),security(("bearer"=[])),responses((status=200,description="Agentes",body=ListaJson<Value>)))]
pub async fn listar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoOrg>,
    Query(q): Query<ConsultaPagina>,
) -> Result<Json<ListaJson<Value>>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let pagina = q.pagina()?;
    let items: Vec<_> = e
        .store
        .agents()
        .list(org, pagina)?
        .iter()
        .map(json_agente)
        .collect();
    let next_cursor = proximo_cursor(&items, pagina.limit, |v| {
        v["id"].as_str().unwrap_or_default().to_owned()
    });
    Ok(Json(ListaJson { items, next_cursor }))
}
#[utoipa::path(get,path="/admin/v1/orgs/{org_id}/agents/{agent_id}",tag="agents",params(("org_id"=String,Path),("agent_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Agente",body=Value)))]
pub async fn mostrar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoAgente>,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let id = id_da_url::<AgentId>(&c.agent_id)?;
    let mut valor = json_agente(&e.store.agents().get(org, id)?);
    valor["product_category_ids"] = json!(
        e.store
            .agents()
            .categories(org, id)?
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    );
    valor["instructions"] = match e.store.instructions().get(org, id) {
        Ok(i) => {
            json!({"agent_id":i.agent_id.to_string(),"org_id":i.org_id.to_string(),"structured":serde_json::from_str::<Value>(&i.structured_json).unwrap_or(json!(i.structured_json)),"markdown":i.markdown,"derived_from_json":i.derived_from_json,"version":i.version,"created_at":i.created_at,"updated_at":i.updated_at})
        }
        Err(poph_store::StoreError::Domain(poph_types::DomainError::NotFound { .. })) => {
            Value::Null
        }
        Err(e) => return Err(e.into()),
    };
    Ok(Json(valor))
}
#[utoipa::path(post,path="/admin/v1/orgs/{org_id}/agents",tag="agents",params(("org_id"=String,Path)),security(("bearer"=[])),request_body=DadosAgente,responses((status=201,description="Agente criado",body=Value)))]
pub async fn criar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoOrg>,
    corpo: Result<Json<DadosAgente>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let Json(d) = corpo.map_err(|_| ApiError::invalido("corpo JSON de agente inválido"))?;
    if d.handoff.is_some()
        || d.jev_json.is_some()
        || d.status.is_some()
        || d.product_category_ids.is_some()
    {
        return Err(ApiError::invalido(
            "campos de alteração não são aceitos na criação",
        ));
    }
    let nome = d
        .name
        .ok_or_else(|| ApiError::validacao("name", "obrigatório"))?;
    if nome.trim().len() < 3 {
        return Err(ApiError::validacao("name", "mínimo de 3 caracteres"));
    }
    let s = d.slug.unwrap_or_else(|| slug(&nome));
    if !(3..=64).contains(&s.len())
        || !s
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(ApiError::validacao(
            "slug",
            "use 3 a 64 caracteres de a-z, 0-9 e hífen",
        ));
    }
    let tipo = d.tipo.unwrap_or_else(|| "custom".into());
    if tipo != "custom" && tipo != "standard" {
        return Err(ApiError::validacao("type", "use custom ou standard"));
    }
    let mut n = NewAgent {
        name: nome,
        slug: s,
        agent_type: tipo,
        ..Default::default()
    };
    if let Some(x) = d.description {
        n.description = x;
    }
    if let Some(x) = d.category {
        n.category = x;
    }
    n.default_department = dep(d.department)?;
    let handoff = dep(d.handoff_department)?;
    if let Some(id) = handoff {
        e.store.departments().get(org, id)?;
    }
    if let Some(x) = d.provider {
        if let Err(erro) = e.config.validate_provider_choice(&x) {
            return Err(ApiError::validacao("provider", erro.to_string()));
        }
        n.provedor = x;
    }
    if let Some(x) = d.model {
        n.modelo = x;
    }
    n.temperatura = d.temperature;
    n.max_tokens_saida = d.max_output_tokens;
    if let Some(x) = d.max_context_tokens {
        n.max_context_tokens = x;
    }
    if let Some(x) = d.grouping_window_ms {
        n.grouping_window_ms = x;
    }
    if let Some(x) = d.max_tool_rounds {
        n.max_tool_rounds = x;
    }
    if let Some(x) = d.max_tool_calls {
        n.max_tools_per_message = x;
    }
    if let Some(x) = d.rag_top_k {
        n.rag_top_k = x;
    }
    if let Some(x) = d.rag_min_score {
        n.rag_cutoff = x;
    }
    let mut a = e.store.agents().create(org, n)?;
    super::auditoria::registrar(&e, &ident, Some(org), "create", "agent", a.id.to_string())?;
    if let Some(h) = handoff {
        a = e.store.agents().update(
            org,
            a.id,
            a.version,
            AgentChange {
                handoff_active: Some(true),
                handoff_department: Some(h),
                ..Default::default()
            },
        )?;
    }
    Ok((StatusCode::CREATED, Json(json_agente(&a))))
}
#[utoipa::path(patch,path="/admin/v1/orgs/{org_id}/agents/{agent_id}",tag="agents",params(("org_id"=String,Path),("agent_id"=String,Path),("If-Match"=String,Header)),security(("bearer"=[])),request_body=DadosAgente,responses((status=200,description="Agente alterado",body=Value),(status=412,description="Conflito",body=crate::erro::RespostaErro)))]
pub async fn atualizar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoAgente>,
    h: HeaderMap,
    corpo: Result<Json<DadosAgente>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let id = id_da_url::<AgentId>(&c.agent_id)?;
    let versao = versao_do_if_match(&h, "agente")?;
    let Json(d) = corpo.map_err(|_| ApiError::invalido("corpo JSON de agente inválido"))?;
    if d.slug.is_some() || d.tipo.is_some() || d.category.is_some() {
        return Err(ApiError::invalido(
            "slug, type e category não podem ser alterados",
        ));
    }
    if let Some(ref j) = d.jev_json {
        serde_json::from_str::<Value>(j)
            .map_err(|_| ApiError::validacao("jev_json", "JSON inválido"))?;
    }
    if let Some(ref escolha) = d.provider
        && let Err(erro) = e.config.validate_provider_choice(escolha)
    {
        return Err(ApiError::validacao("provider", erro.to_string()));
    }
    let categorias = d
        .product_category_ids
        .map(|v| {
            v.iter()
                .map(|s| id_da_url::<ProductCategoryId>(s))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    let m = AgentChange {
        name: d.name,
        description: d.description,
        status: d.status.as_deref().map(status_pedido).transpose()?,
        default_department: dep(d.department)?,
        handoff_active: d.handoff,
        handoff_department: dep(d.handoff_department)?,
        provedor: d.provider,
        modelo: d.model,
        temperatura: d.temperature,
        max_tokens_saida: d.max_output_tokens,
        max_context_tokens: d.max_context_tokens,
        grouping_window_ms: d.grouping_window_ms,
        max_tool_rounds: d.max_tool_rounds,
        max_tools_per_message: d.max_tool_calls,
        rag_top_k: d.rag_top_k,
        rag_cutoff: d.rag_min_score,
        jev_json: d.jev_json,
    };
    let tem = m.name.is_some()
        || m.description.is_some()
        || m.status.is_some()
        || m.default_department.is_some()
        || m.handoff_active.is_some()
        || m.handoff_department.is_some()
        || m.provedor.is_some()
        || m.modelo.is_some()
        || m.temperatura.is_some()
        || m.max_tokens_saida.is_some()
        || m.max_context_tokens.is_some()
        || m.grouping_window_ms.is_some()
        || m.max_tool_rounds.is_some()
        || m.max_tools_per_message.is_some()
        || m.rag_top_k.is_some()
        || m.rag_cutoff.is_some()
        || m.jev_json.is_some();
    if !tem && categorias.is_none() {
        return Err(ApiError::invalido("nada a alterar"));
    }
    let mut a = if tem {
        e.store.agents().update(org, id, versao, m)?
    } else {
        let a = e.store.agents().get(org, id)?;
        if a.version != versao {
            return Err(ApiError::conflito_de_versao(
                "agente",
                &id.to_string(),
                versao,
                a.version,
            ));
        }
        a
    };
    if tem {
        super::auditoria::registrar(&e, &ident, Some(org), "update", "agent", id.to_string())?;
    }
    if let Some(cats) = categorias {
        a = e
            .store
            .agents()
            .define_categories(org, id, a.version, &cats)?;
        super::auditoria::registrar(&e, &ident, Some(org), "update", "agent", id.to_string())?;
    }
    Ok(Json(json_agente(&a)))
}
#[utoipa::path(delete,path="/admin/v1/orgs/{org_id}/agents/{agent_id}",tag="agents",params(("org_id"=String,Path),("agent_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Agente excluído",body=Value)))]
pub async fn excluir(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoAgente>,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let id = id_da_url::<AgentId>(&c.agent_id)?;
    e.store.agents().delete(org, id)?;
    super::auditoria::registrar(&e, &ident, Some(org), "delete", "agent", id.to_string())?;
    Ok(Json(json!({"removido":true,"agent_id":id.to_string()})))
}
