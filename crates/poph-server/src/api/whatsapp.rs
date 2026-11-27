use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
};
use poph_evolution::{EvolutionClient, EvolutionError};
use poph_store::AdminIntegration;
use poph_types::{AgentId, IntegrationId, OrgId};
use serde::Deserialize;
use serde_json::{Value, json};

use super::if_match_version;
use crate::{
    auth::Identidade,
    erro::{ApiError, id_da_url as parse_path_id},
    estado::Estado,
};

#[derive(Deserialize)]
pub struct PathParams {
    org_id: String,
    agent_id: String,
    integration_id: String,
}

#[doc = " Corpo opcional do pareamento."]
#[derive(Deserialize, utoipa::ToSchema, Default)]
#[serde(deny_unknown_fields)]
#[schema(as = DadosPareamento)]
pub struct PairingData {
    #[doc = " Número do WhatsApp que vai parear, só dígitos com DDI (`5511999998888`). Sem ele, o"]
    #[doc = " pareamento é pelo QR; com ele, a Evolution também devolve o código de 8 caracteres."]
    number: Option<String>,
}

struct Target {
    org: OrgId,
    id: IntegrationId,
    integration: AdminIntegration,
    instancia: String,
    client: EvolutionClient,
}

impl Target {
    fn id(&self) -> &str {
        &self.integration.id
    }
}

fn target(e: &Estado, ident: &Identidade, c: &PathParams) -> Result<Target, ApiError> {
    let org: OrgId = parse_path_id(&c.org_id)?;
    ident.exigir_org(org)?;
    let agent_id: AgentId = parse_path_id(&c.agent_id)?;
    let id: IntegrationId = parse_path_id(&c.integration_id)?;
    let integration = e.store.get_integration(org, agent_id, id)?;
    if !integration.kind.trim().eq_ignore_ascii_case("whatsapp") {
        return Err(ApiError::novo(
            StatusCode::CONFLICT,
            "integracao_nao_e_whatsapp",
            format!(
                "esta integração é do tipo `{}`; só a de tipo `whatsapp` tem pareamento, estado e desconexão",
                integration.kind
            ),
        )
        .com_detalhes(json!({ "kind": integration.kind })));
    }
    let channel = e.canal_evolution_da_integracao(org, id).ok_or_else(|| {
        ApiError::novo(
            StatusCode::CONFLICT,
            "whatsapp_nao_configurado",
            "esta integração não tem instância da Evolution declarada na configuração \
                 do servidor (`[evolution].instancias`: org, instância e integração)",
        )
        .com_detalhes(json!({ "integration_id": id.to_string() }))
    })?;
    let client = channel.cliente.clone().ok_or_else(|| {
        ApiError::novo(
            StatusCode::SERVICE_UNAVAILABLE,
            "evolution_indisponivel",
            "o servidor não tem como falar com a Evolution: falta `EVOLUTION_API_KEY` no ambiente",
        )
    })?;
    Ok(Target {
        org,
        id,
        integration,
        instancia: channel.instancia.clone(),
        client,
    })
}

fn check_version(target: &Target, h: &HeaderMap) -> Result<(), ApiError> {
    let version = if_match_version(h, "integração")?;
    if version != target.integration.version {
        return Err(ApiError::conflito_de_versao(
            "integração",
            target.id(),
            version,
            target.integration.version,
        ));
    }
    Ok(())
}

fn write_state(e: &Estado, target: &Target, state: &str) -> Result<(), ApiError> {
    e.store
        .set_integration_connection_state(target.org, target.id, state)?;
    Ok(())
}

fn evolution_error(error: EvolutionError, operation: &str) -> ApiError {
    match &error {
        EvolutionError::Network(_) => ApiError::novo(
            StatusCode::SERVICE_UNAVAILABLE,
            "evolution_indisponivel",
            format!("não consegui falar com a Evolution em `{operation}`; tente de novo"),
        ),
        EvolutionError::Response { status, message } => ApiError::novo(
            StatusCode::BAD_GATEWAY,
            "evolution_recusou",
            format!("a Evolution recusou `{operation}`: {message}"),
        )
        .com_detalhes(json!({ "evolution_status": status })),
        EvolutionError::InvalidResponse(reason) => ApiError::novo(
            StatusCode::BAD_GATEWAY,
            "evolution_resposta_invalida",
            format!("a resposta da Evolution não serve para `{operation}`: {reason}"),
        ),
        other => ApiError::novo(
            StatusCode::BAD_GATEWAY,
            "evolution_falhou",
            format!("falha em `{operation}` com a Evolution: {other}"),
        ),
    }
}

fn instance_already_exists(error: &EvolutionError) -> bool {
    match error {
        EvolutionError::Response { status, message } => {
            let text = message.to_ascii_lowercase();
            (*status == 403 || *status == 409 || *status == 422)
                && ["already", "in use", "existe"]
                    .iter()
                    .any(|pista| text.contains(pista))
        }
        _ => false,
    }
}

fn connection_state(raw: &str) -> &'static str {
    match raw {
        "open" => "connected",
        "connecting" => "awaiting_scan",
        "close" | "closed" => "disconnected",
        _ => "desconhecido",
    }
}

async fn optional_state(client: &EvolutionClient) -> Option<String> {
    client
        .connection_state()
        .await
        .ok()
        .map(|state| state.trim().to_ascii_lowercase())
}

#[utoipa::path(operation_id = "qrcode", post,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/qrcode",tag="integrations",params(("org_id"=String,Path),("agent_id"=String,Path),("integration_id"=String,Path),("If-Match"=String,Header)),security(("bearer"=[])),request_body=PairingData,responses((status=200,description="QR de pareamento",body=Value),(status=412,description="Conflito de versão",body=crate::erro::RespostaErro)))]
pub async fn qrcode(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<PathParams>,
    h: HeaderMap,
    body: Result<Json<PairingData>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let target = target(&e, &ident, &c)?;
    check_version(&target, &h)?;
    let number = body
        .ok()
        .and_then(|Json(data)| data.number)
        .map(|number| number.trim().to_owned())
        .filter(|number| !number.is_empty());

    if optional_state(&target.client).await.as_deref() == Some("open") {
        write_state(&e, &target, "connected")?;
        return Ok(Json(json!({
            "integration_id": target.id(),
            "instance": target.instancia,
            "state": "open",
            "connection_state": "connected",
            "qrcode": Value::Null,
            "pairing_code": Value::Null,
        })));
    }

    match target.client.create_instance().await {
        Ok(_) => {}
        Err(error) if instance_already_exists(&error) => {}
        Err(error) => return Err(evolution_error(error, "criar instância")),
    }
    let pairing = target
        .client
        .connect(number.as_deref())
        .await
        .map_err(|error| evolution_error(error, "parear"))?;

    if pairing.qr_base64.is_none() && pairing.qr_text.is_none() && pairing.pairing_code.is_none() {
        return Err(ApiError::novo(
            StatusCode::BAD_GATEWAY,
            "evolution_sem_qr",
            "a Evolution respondeu o pareamento sem QR e sem código; peça de novo em alguns segundos",
        ));
    }

    write_state(&e, &target, "awaiting_scan")?;
    super::audit::register(
        &e,
        &ident,
        Some(target.org),
        "whatsapp_qrcode",
        "integration",
        target.id().to_owned(),
    )?;
    Ok(Json(json!({
        "integration_id": target.id(),
        "instance": target.instancia,
        "state": "connecting",
        "connection_state": "awaiting_scan",
        "qrcode": {
            "base64": pairing.qr_base64,
            "code": pairing.qr_text,
            "count": pairing.counter,
        },
        "expira_em": Value::Null,
        "pairing_code": pairing.pairing_code,
    })))
}

#[utoipa::path(operation_id = "estado", get,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/state",tag="integrations",params(("org_id"=String,Path),("agent_id"=String,Path),("integration_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Estado da conexão",body=Value)))]
pub async fn status(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<PathParams>,
) -> Result<Json<Value>, ApiError> {
    let target = target(&e, &ident, &c)?;
    let raw = target
        .client
        .connection_state()
        .await
        .map_err(|error| evolution_error(error, "estado"))?
        .trim()
        .to_ascii_lowercase();
    let normalized = connection_state(&raw);
    write_state(&e, &target, normalized)?;
    Ok(Json(json!({
        "integration_id": target.id(),
        "instance": target.instancia,
        "state": raw,
        "connection_state": normalized,
    })))
}

#[utoipa::path(operation_id = "desconectar", post,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/disconnect",tag="integrations",params(("org_id"=String,Path),("agent_id"=String,Path),("integration_id"=String,Path),("If-Match"=String,Header)),security(("bearer"=[])),responses((status=200,description="Instância desconectada",body=Value),(status=412,description="Conflito de versão",body=crate::erro::RespostaErro)))]
pub async fn disconnect(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<PathParams>,
    h: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let target = target(&e, &ident, &c)?;
    check_version(&target, &h)?;
    target
        .client
        .desconectar()
        .await
        .map_err(|error| evolution_error(error, "desconectar"))?;
    write_state(&e, &target, "disconnected")?;
    super::audit::register(
        &e,
        &ident,
        Some(target.org),
        "whatsapp_disconnect",
        "integration",
        target.id().to_owned(),
    )?;
    Ok(Json(json!({
        "integration_id": target.id(),
        "instance": target.instancia,
        "connection_state": "disconnected",
        "disconnected": true,
    })))
}
