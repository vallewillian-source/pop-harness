use std::time::SystemTime;

use axum::Json;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use poph_store::{OutboxState, Page};
use serde::Serialize;
use utoipa::ToSchema;

use crate::state::ServerState;

#[doc = " Resposta do `/healthz`."]
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct HealthJson {
    #[doc = " `ok` quando os bancos respondem; `degraded` quando não."]
    pub status: String,
    #[doc = " Versão do binário."]
    pub version: String,
    #[doc = " Segundos desde a subida."]
    pub uptime_seconds: u64,
    #[doc = " Versão de esquema de cada banco."]
    pub databases: DatabaseVersions,
    #[doc = " Nome desta instância no `cluster.toml` (HR-50b-2). Ausente no modo instância única."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    #[doc = " Impressão digital do **mapa** do cluster em uso, sem a instância local (HR-50b-2): é o valor"]
    #[doc = " que duas instâncias comparam entre si. Ausente no modo instância única."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cluster_map_fingerprint: Option<String>,
}

#[doc = " Versões de esquema lidas do disco."]
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = VersaoBancos)]
pub struct DatabaseVersions {
    #[doc = " Versão do `harness.db`."]
    pub harness: u32,
    #[doc = " Versão do `rag.db`."]
    pub rag: u32,
}

#[doc = " Resumo de métricas (forma JSON; o `/metrics` devolve texto Prometheus)."]
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = MetricasJson)]
pub struct MetricsJson {
    #[doc = " RAM residente do processo, em bytes."]
    pub memory_bytes: i64,
    #[doc = " Turnos esperando na fila de entrada."]
    pub queued_turns: i64,
    #[doc = " Mensagens esperando envio."]
    pub queued_outbox: i64,
    #[doc = " Agentes com configuração carregada na memória."]
    pub loaded_agents: i64,
    #[doc = " Segundos desde a subida."]
    pub uptime_seconds: u64,
}

#[doc = " `GET /healthz`."]
#[utoipa::path(
    get,
    path = "/healthz",
    tag = "operacao",
    operation_id = "operacao_healthz",
    responses(
        (status = 200, description = "Servidor e bancos ok", body = HealthJson),
        (status = 503, description = "Banco indisponível", body = HealthJson)
    )
)]
pub async fn healthz(State(state): State<ServerState>) -> Response {
    let harness = poph_store::file_version(state.store.harness_path())
        .ok()
        .flatten()
        .unwrap_or(0);
    let rag = poph_store::file_version(state.store.rag_path())
        .ok()
        .flatten()
        .unwrap_or(0);
    let database_ok = state.store.tables().is_ok();
    let cluster = state.cluster();
    let (instance_id, cluster_map_fingerprint) = if cluster.declares_cluster() {
        (
            Some(cluster.config.instance_id.trim().to_owned()),
            Some(cluster.config.map_fingerprint()),
        )
    } else {
        (None, None)
    };
    let body = HealthJson {
        status: if database_ok { "ok" } else { "degraded" }.to_owned(),
        version: crate::VERSAO.to_owned(),
        uptime_seconds: seconds_up(&state),
        databases: DatabaseVersions { harness, rag },
        instance_id,
        cluster_map_fingerprint,
    };
    let status = if database_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(body)).into_response()
}

#[doc = " `GET /metrics` (texto de exposição do Prometheus)."]
#[utoipa::path(
    get,
    path = "/metrics",
    tag = "operacao",
    operation_id = "operacao_metrics",
    responses((status = 200, description = "Métricas em texto Prometheus", body = String))
)]
pub async fn metrics(State(state): State<ServerState>) -> Response {
    refresh_meters(&state);
    let text = state.registry.render();
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        text,
    )
        .into_response()
}

#[doc = " `GET /openapi.json` — o mesmo documento que está em `/admin/v1/openapi.json`."]
#[utoipa::path(
    get,
    path = "/openapi.json",
    tag = "operacao",
    operation_id = "operacao_openapi",
    responses((status = 200, description = "Documento OpenAPI 3.1", body = Object))
)]
pub async fn openapi() -> Response {
    let document = super::document();
    let json = serde_json::to_value(&document).unwrap_or_else(|error| {
        tracing::error!(error = %error, "falha ao serializar o OpenAPI");
        serde_json::json!({ "erro": "não consegui serializar o OpenAPI" })
    });
    (StatusCode::OK, Json(json)).into_response()
}

fn refresh_meters(state: &ServerState) {
    if let Some(fluxos) = state.flow_metrics() {
        fluxos.sync_cache(state.flow_cache.metrics());
    }
    state
        .metrics
        .process_memory_bytes
        .set(resident_memory_bytes() as i64);
    state
        .metrics
        .harness_db_bytes
        .set(size(state.store.harness_path()) as i64);
    state
        .metrics
        .rag_db_bytes
        .set(size(state.store.rag_path()) as i64);

    let mut pending_entry = 0i64;
    let mut pending_output = 0i64;
    for org in all_orgs(state) {
        match state.store.inbox().count_pending(org) {
            Ok(count) => pending_entry += count as i64,
            Err(error) => tracing::warn!(error = %error, "métrica: não contei a entrada"),
        }
        match state
            .store
            .outbox()
            .count_by_state(org, OutboxState::Pending)
        {
            Ok(count) => pending_output += count as i64,
            Err(error) => tracing::warn!(error = %error, "métrica: não contei a saída"),
        }
    }
    state.metrics.turn_queue.set(pending_entry);
    state.metrics.outbox_queue.set(pending_output);

    state.metrics.loaded_agents.set(0);
}

fn all_orgs(state: &ServerState) -> Vec<poph_types::OrgId> {
    let mut ids = Vec::new();
    let mut after = None;
    loop {
        let page = Page { after, limit: 200 };
        match state.store.orgs().list(page) {
            Ok(orgs) => {
                let quantity = orgs.len();
                if let Some(last) = orgs.last() {
                    after = Some(last.id.as_uuid());
                }
                ids.extend(orgs.iter().map(|org| org.id));
                if quantity < 200 {
                    break;
                }
            }
            Err(error) => {
                tracing::warn!(error = %error, "métrica: não listei as orgs");
                break;
            }
        }
    }
    ids
}

fn seconds_up(state: &ServerState) -> u64 {
    SystemTime::now()
        .duration_since(state.started_at)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn size(path: &std::path::Path) -> u64 {
    std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

fn resident_memory_bytes() -> u64 {
    let Ok(text) = std::fs::read_to_string("/proc/self/status") else {
        return 0;
    };
    for line in text.lines() {
        if let Some(remainder) = line.strip_prefix("VmRSS:") {
            let number: u64 = remainder
                .trim()
                .trim_end_matches(" kB")
                .trim()
                .parse()
                .unwrap_or(0);
            return number.saturating_mul(1024);
        }
    }
    0
}

#[allow(dead_code)]
pub(crate) fn summary(state: &ServerState) -> MetricsJson {
    MetricsJson {
        memory_bytes: resident_memory_bytes() as i64,
        queued_turns: state.metrics.turn_queue.value(),
        queued_outbox: state.metrics.outbox_queue.value(),
        loaded_agents: state.metrics.loaded_agents.value(),
        uptime_seconds: seconds_up(state),
    }
}
