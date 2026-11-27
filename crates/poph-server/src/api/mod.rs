mod agent_flows;
mod agents;
mod analytics;
mod audit;
mod backup;
mod cases;
mod categories;
mod cluster;
mod content;
mod conversation_analytics;
mod conversations;
mod departments;
mod flows;
mod health;
mod instructions;
mod integrations;
mod knowledge;
mod library;
mod manifests;
mod matrix;
pub mod matrix_as;
mod orgs;
mod pagination;
mod products;
mod site_chat;
mod sla_analytics;
pub mod test_chat;
mod tokens;
mod usage;
mod whatsapp;

use axum::Router;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::IntoResponse;
use axum::routing::get;
use poph_store::Status;
use serde_json::json;

use crate::erro::ApiError;
use crate::estado::Estado;

pub use backup::{LIMIT_IMPORT, LIMIT_IMPORT_BYTES};
pub use health::{HealthJson, MetricsJson};
pub use pagination::{ListJson, PageQuery, next_cursor};

pub(crate) fn tool_parts(raw: &str) -> (serde_json::Value, serde_json::Value) {
    let empty = || serde_json::Value::Array(Vec::new());
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(serde_json::Value::Array(names)) => (serde_json::Value::Array(names), empty()),
        Ok(serde_json::Value::Object(map)) => (
            map.get("nomes").cloned().unwrap_or_else(empty),
            map.get("chamadas").cloned().unwrap_or_else(empty),
        ),
        _ => (empty(), empty()),
    }
}

pub fn router(state: Estado) -> Router {
    let admin = Router::new()
        .route("/admin/v1/orgs", get(orgs::list).post(orgs::create))
        .route(
            "/admin/v1/orgs/{org_id}",
            get(orgs::show)
                .patch(orgs::update)
                .delete(orgs::delete),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents",
            get(agents::list).post(agents::create),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}",
            get(agents::show)
                .patch(agents::update)
                .delete(agents::delete),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/flows",
            get(agent_flows::list).put(agent_flows::replace),
        )
        .route(
            "/admin/v1/orgs/{org_id}/flows/validate",
            axum::routing::post(flows::validate_script),
        )
        .route(
            "/admin/v1/orgs/{org_id}/flows/format",
            axum::routing::post(flows::format),
        )
        .route(
            "/admin/v1/orgs/{org_id}/flows",
            get(flows::list).post(flows::create),
        )
        .route(
            "/admin/v1/orgs/{org_id}/flows/{flow_id}",
            get(flows::show)
                .patch(flows::update)
                .delete(flows::delete),
        )
        .route(
            "/admin/v1/orgs/{org_id}/flows/{flow_id}/publish",
            axum::routing::post(flows::publish),
        )
        .route(
            "/admin/v1/orgs/{org_id}/flows/{flow_id}/revisions",
            get(flows::list_revisions),
        )
        .route(
            "/admin/v1/orgs/{org_id}/flows/{flow_id}/revisions/{number}",
            get(flows::show_revision),
        )
        .route(
            "/admin/v1/orgs/{org_id}/flows/{flow_id}/diff",
            get(flows::diff),
        )
        .route(
            "/admin/v1/orgs/{org_id}/flows/{flow_id}/preview",
            get(flows::preview),
        )
        .route("/admin/v1/orgs/{org_id}/fichas", get(cases::list))
        .route(
            "/admin/v1/orgs/{org_id}/fichas/{ficha_id}",
            get(cases::show),
        )
        .route(
            "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/events",
            get(cases::events),
        )
        .route(
            "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/flow-runs",
            axum::routing::post(cases::start_flow),
        )
        .route(
            "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/flow-runs/{run_id}/actions",
            axum::routing::post(cases::apply_action),
        )
        .route(
            "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/flow-runs/{run_id}/abandon",
            axum::routing::post(cases::abandon),
        )
        .route(
            "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/close",
            axum::routing::post(cases::close),
        )
        .route("/admin/v1/orgs/{org_id}/analytics/conversations", get(conversation_analytics::report))
        .route("/admin/v1/orgs/{org_id}/analytics/sla", get(sla_analytics::report))
        .route("/admin/v1/orgs/{org_id}/usage/tokens", get(usage::tokens))
        .route(
            "/admin/v1/orgs/{org_id}/analytics/operational-summary",
            get(analytics::operational_summary),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions",
            get(instructions::list).post(instructions::create),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions/current",
            get(instructions::show)
                .patch(instructions::update)
                .delete(instructions::delete),
        )
        .route(
            "/admin/v1/orgs/{org_id}/knowledge",
            get(knowledge::list).post(knowledge::create),
        )
        .route(
            "/admin/v1/orgs/{org_id}/knowledge/{group_id}",
            get(knowledge::show)
                .patch(knowledge::update)
                .delete(knowledge::delete),
        )
        .route(
            "/admin/v1/orgs/{org_id}/knowledge/{group_id}/items",
            get(knowledge::list_items).post(knowledge::create_item),
        )
        .route(
            "/admin/v1/orgs/{org_id}/knowledge/{group_id}/items/{item_id}",
            get(knowledge::show_item)
                .patch(knowledge::update_item)
                .delete(knowledge::delete_item),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/knowledge/{group_id}",
            axum::routing::post(knowledge::link).delete(knowledge::unlink),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/knowledge/reindex",
            axum::routing::post(knowledge::reindex),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations",
            get(integrations::list).post(integrations::create),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}",
            get(integrations::show)
                .patch(integrations::update)
                .delete(integrations::delete),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/qrcode",
            axum::routing::post(whatsapp::qrcode),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/state",
            get(whatsapp::status),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/disconnect",
            axum::routing::post(whatsapp::disconnect),
        )
        .route(
            "/admin/v1/orgs/{org_id}/backup",
            get(backup::export),
        )
        .route(
            "/admin/v1/orgs/{org_id}/backup/import",
            axum::routing::post(backup::import).layer(
                axum::extract::DefaultBodyLimit::max(backup::LIMIT_IMPORT),
            ),
        )
        .route(
            "/admin/v1/cluster/reload",
            axum::routing::post(cluster::reload),
        )
        .route(
            "/admin/v1/orgs/{org_id}/manifests",
            get(manifests::list),
        )
        .route(
            "/admin/v1/orgs/{org_id}/manifests/{kind}",
            get(manifests::show),
        )
        .route(
            "/admin/v1/orgs/{org_id}/tokens",
            get(tokens::list).post(tokens::create),
        )
        .route(
            "/admin/v1/orgs/{org_id}/tokens/{token_id}",
            get(tokens::show).delete(tokens::revoke),
        )
        .route(
            "/admin/v1/orgs/{org_id}/categories",
            get(categories::list).post(categories::create),
        )
        .route(
            "/admin/v1/orgs/{org_id}/categories/{category_id}",
            get(categories::show)
                .patch(categories::update)
                .delete(categories::delete),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/files",
            get(library::list).post(library::create),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/files/{file_id}",
            get(library::show).delete(library::delete),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/files/{file_id}/content",
            get(library::content),
        )
        .route(
            "/admin/v1/orgs/{org_id}/conversations",
            get(conversations::list),
        )
        .route(
            "/admin/v1/orgs/{org_id}/conversations/{conversation_id}",
            get(conversations::show),
        )
        .route(
            "/admin/v1/orgs/{org_id}/conversations/{conversation_id}/messages",
            get(conversations::messages),
        )
        .route(
            "/admin/v1/orgs/{org_id}/conversations/{conversation_id}/turns",
            get(conversations::turns),
        )
        .route(
            "/admin/v1/orgs/{org_id}/conversations/{conversation_id}/reopen",
            axum::routing::post(conversations::reopen),
        )
        .route(
            "/admin/v1/orgs/{org_id}/conversations/{conversation_id}/close",
            axum::routing::post(conversations::close),
        )
        .route(
            "/admin/v1/orgs/{org_id}/conversations/{conversation_id}/transfer",
            axum::routing::post(conversations::transfer),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat",
            get(test_chat::list).post(test_chat::create),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}",
            get(test_chat::history),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}/messages",
            axum::routing::post(test_chat::send_message)
                .layer(axum::extract::DefaultBodyLimit::max(24 * 1024 * 1024)),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}/reopen",
            axum::routing::post(test_chat::reopen),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}/messages/{message_id}/attachments/{attachment_id}/content",
            get(test_chat::attachment_content),
        )
        .route(
            "/admin/v1/orgs/{org_id}/products",
            get(products::list).post(products::create),
        )
        .route(
            "/admin/v1/orgs/{org_id}/products/{product_id}",
            get(products::show)
                .patch(products::update)
                .delete(products::delete),
        )
        .route(
            "/admin/v1/orgs/{org_id}/departments",
            get(departments::list).post(departments::create),
        )
        .route(
            "/admin/v1/orgs/{org_id}/departments/{department_id}",
            get(departments::show)
                .patch(departments::update)
                .delete(departments::delete),
        )
        .route(
            "/admin/v1/orgs/{org_id}/matrix",
            get(matrix::show).put(matrix::update),
        )
        .route(
            "/admin/v1/orgs/{org_id}/matrix/spaces",
            get(matrix::list_space).put(matrix::enable_space),
        )
        .route(
            "/admin/v1/orgs/{org_id}/matrix/spaces/{department_id}",
            axum::routing::delete(matrix::disable_space),
        )
        .merge(matrix_as::routes())
        .merge(site_chat::routes())
        .with_state(state.clone());

    Router::new()
        .merge(admin)
        .route("/healthz", get(health::healthz))
        .route("/metrics", get(health::metrics))
        .route(
            "/webhook/evolution/{org_id}/{instancia}",
            axum::routing::post(crate::evolution::receber).layer(
                axum::extract::DefaultBodyLimit::max(poph_evolution::DEFAULT_BODY_CAP as usize),
            ),
        )
        .route("/openapi.json", get(health::openapi))
        .route("/admin/v1/openapi.json", get(health::openapi))
        .fallback(console_page_or_404)
        .method_not_allowed_fallback(method_not_allowed)
        .with_state(state.clone())
        .layer(axum::middleware::from_fn_with_state(
            state,
            crate::encaminhamento::middleware,
        ))
}

async fn missing_route(uri: Uri) -> ApiError {
    let path = uri.path().to_owned();
    ApiError::novo(
        StatusCode::NOT_FOUND,
        "not_found",
        format!("a rota `{path}` não existe"),
    )
    .com_detalhes(json!({ "path": path }))
}

async fn console_page_or_404(method: Method, uri: Uri) -> axum::response::Response {
    if method == Method::GET || method == Method::HEAD {
        return crate::console::servir(&uri, &method);
    }
    missing_route(uri).await.into_response()
}

async fn method_not_allowed() -> ApiError {
    ApiError::novo(
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        "método não permitido nesta rota",
    )
}

pub fn if_match_version(headers: &HeaderMap, entity: &str) -> Result<u64, ApiError> {
    let value = headers
        .get(axum::http::header::IF_MATCH)
        .ok_or_else(|| ApiError::if_match_obrigatorio(entity))?;
    let text = value
        .to_str()
        .map_err(|_| ApiError::invalido("If-Match não é texto ASCII"))?;
    let clean = text
        .trim()
        .trim_start_matches("W/")
        .trim_matches('"')
        .trim();
    clean.parse::<u64>().map_err(|_| {
        ApiError::invalido(format!(
            "If-Match `{text}` não é uma versão numérica (ex.: 3)"
        ))
        .com_detalhes(serde_json::json!({ "header": "If-Match", "value": text }))
    })
}

pub fn request_status(text: &str) -> Result<Status, ApiError> {
    match text {
        "active" => Ok(Status::Active),
        "inactive" => Ok(Status::Inactive),
        "archived" => Ok(Status::Archived),
        other => Err(ApiError::validacao(
            "status",
            format!("`{other}` não é uma situação; use active, inactive ou archived"),
        )),
    }
}

pub fn status_text(status: Status) -> &'static str {
    status.as_text()
}

pub fn document() -> utoipa::openapi::OpenApi {
    use utoipa::OpenApi as _;
    OpenApiDoc::openapi()
}

#[derive(utoipa::OpenApi)]
#[openapi(
    info(
        title = "POP Harness — API administrativa",
        description = "API `/admin/v1` do motor de agentes multi-tenant. Autenticação por token \
                       (`Authorization: Bearer`), escopo por org, erros em `{code, message, details}` \
                       e paginação por cursor.",
        version = "0.1.0"
    ),
    paths(
        integrations::list, integrations::show, integrations::create, integrations::update, integrations::delete,
        manifests::list, manifests::show,
        cluster::reload,
        backup::export, backup::import,
        whatsapp::qrcode, whatsapp::status, whatsapp::disconnect,
        knowledge::link, knowledge::unlink, knowledge::reindex,
        knowledge::list_items, knowledge::show_item, knowledge::create_item, knowledge::update_item, knowledge::delete_item,
        knowledge::list, knowledge::show, knowledge::create, knowledge::update, knowledge::delete,
        agents::list, agents::show, agents::create, agents::update, agents::delete,
        agent_flows::list, agent_flows::replace,
        instructions::list, instructions::show, instructions::create, instructions::update, instructions::delete,
        orgs::list,
        orgs::create,
        orgs::show,
        orgs::update,
        orgs::delete,
        tokens::list,
        tokens::create,
        tokens::show,
        tokens::revoke,
        categories::list,
        categories::create,
        categories::show,
        categories::update,
        categories::delete,
        departments::list,
        departments::create,
        departments::show,
        departments::update,
        departments::delete,
        library::create,
        library::list,
        library::show,
        library::content,
        library::delete,
        conversations::list,
        conversations::show,
        conversations::messages,
        conversations::turns,
        conversations::reopen,
        conversations::close,
        conversations::transfer,
        matrix::show,
        matrix::update,
        matrix::list_space,
        matrix::enable_space,
        matrix::disable_space,
        test_chat::create,
        test_chat::list,
        test_chat::history,
        test_chat::reopen,
        test_chat::send_message,
        test_chat::attachment_content,
        products::create,
        products::update,
        products::delete,
        products::list,
        products::show,
        flows::validate_script, flows::list, flows::create, flows::show, flows::update,
        flows::delete, flows::publish, flows::list_revisions, flows::show_revision,
        flows::diff, flows::preview, flows::format,
        cases::list, cases::show, cases::events, cases::start_flow, cases::apply_action,
        cases::abandon, cases::close,
        site_chat::create,
        site_chat::submit,
        site_chat::turn,
        site_chat::cancel,
        site_chat::events,
        site_chat::events_stream,
        site_chat::history,
        analytics::operational_summary,
        conversation_analytics::report,
        sla_analytics::report,
        usage::tokens,
        health::healthz,
        health::metrics,
    ),
    components(schemas(
        integrations::Dados,
        whatsapp::PairingData,
        knowledge::ItemData,
        knowledge::Dados,
        agents::AgentData,
        instructions::Dados,
        crate::erro::RespostaErro,
        orgs::OrgJson,
        orgs::CreateOrg,
        orgs::UpdateOrg,
        tokens::TokenJson,
        tokens::CreateToken,
        tokens::TokenCreatedJson,
        products::ProductData,
        categories::CategoryJson,
        categories::CreateCategory,
        categories::UpdateCategory,
        departments::DepartmentJson,
        departments::CreateDepartment,
        departments::UpdateDepartment,
        pagination::ListJson<orgs::OrgJson>,
        orgs::InstanceStatusJson,
        pagination::ListJson<tokens::TokenJson>,
        pagination::ListJson<categories::CategoryJson>,
        pagination::ListJson<departments::DepartmentJson>,
        library::FileJson,
        pagination::ListJson<library::FileJson>,
        test_chat::TestMessageData,
        site_chat::CreateBody,
        site_chat::MessageBody,
        matrix::MatrixStateJson,
        matrix::DelayJson,
        matrix::SpaceJson,
        matrix::SpacesJson,
        matrix::EnableDisable,
        matrix::EnableSpace,
        matrix::SpaceDisabledJson,
        health::HealthJson,
        health::MetricsJson,
        sla_analytics::SlaReportJson, sla_analytics::AgeBucket, sla_analytics::StaleItem,
        conversation_analytics::ConversationReportJson, conversation_analytics::SummaryJson, conversation_analytics::DayJson, conversation_analytics::AgentJson, conversation_analytics::ChannelJson,
        analytics::OperationalSummaryJson,
        analytics::NodeCountJson,
        usage::TokenUsageJson, usage::AgentUsageJson, usage::DayUsageJson,
    )),
    tags(
        (name = "orgs", description = "Orgs (tenants)"),
        (name = "tokens", description = "Tokens de acesso à API"),
        (name = "categories", description = "Categorias de produto"),
        (name = "departments", description = "Departamentos"),
        (name = "test-chat", description = "Chat de teste do Console (SSE e anexos)"),
        (name = "operacao", description = "Saúde, métricas e contrato"),
        (name = "backup", description = "Exportar e importar a organização num arquivo único"),
        (name = "cluster", description = "Roteamento estático entre instâncias (cluster.toml)"),
        (name = "matrix", description = "Módulo Matrix: estado por org e espaços por departamento"),
        (name = "flows", description = "Fluxos: script, validação, publicação, revisões, diff e preview"),
        (name = "fichas", description = "Fichas do atendimento, execuções de fluxo e ações humanas"),
        (name = "usage", description = "Uso agregado de tokens"),
        (name = "analytics", description = "Leitura agregada do atendimento: contadores e execuções ativas por nó"),
    ),
    modifiers(&SecurityScheme)
)]
struct OpenApiDoc;

struct SecurityScheme;

impl utoipa::Modify for SecurityScheme {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
        );
    }
}

#[cfg(test)]
mod tool_tests {
    use super::tool_parts;
    use serde_json::json;

    #[test]
    fn current_format_separates_names_and_calls() {
        let raw = json!({
            "nomes": ["consultar_base"],
            "chamadas": [{"nome": "consultar_base", "estado": "executada"}]
        })
        .to_string();
        let (names, call) = tool_parts(&raw);
        assert_eq!(names, json!(["consultar_base"]));
        assert_eq!(call.as_array().expect("lista").len(), 1);
    }

    #[test]
    fn old_turn_with_name_list_reads_without_error() {
        let (names, call) = tool_parts("[\"buscar_produto\",\"consultar_base\"]");
        assert_eq!(names, json!(["buscar_produto", "consultar_base"]));
        assert_eq!(call, json!([]));
    }

    #[test]
    fn unreadable_or_empty_value_becomes_empty_list() {
        for raw in ["", "não é json", "null", "{}"] {
            let (names, call) = tool_parts(raw);
            assert_eq!(names, json!([]), "nomes de `{raw}`");
            assert_eq!(call, json!([]), "chamadas de `{raw}`");
        }
    }
}
