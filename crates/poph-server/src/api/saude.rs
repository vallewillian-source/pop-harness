//! Rotas de operação: `/healthz`, `/metrics` e o documento OpenAPI.
//!
//! `/healthz` e `/metrics` ficam fora do `/admin/v1` e **sem autenticação**: o servidor escuta só
//! na própria máquina por padrão (configuração `servidor.endereco`), e as duas rotas não devolvem
//! dado de cliente. Se o endereço mudar para a rede, o proxy é que decide quem alcança.

use std::time::SystemTime;

use axum::Json;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use poph_store::{EstadoSaida, Pagina};
use serde::Serialize;
use utoipa::ToSchema;

use crate::estado::Estado;

/// Resposta do `/healthz`.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct HealthJson {
    /// `ok` quando os bancos respondem; `degraded` quando não.
    pub status: String,
    /// Versão do binário.
    pub version: String,
    /// Segundos desde a subida.
    pub uptime_seconds: u64,
    /// Versão de esquema de cada banco.
    pub databases: VersaoBancos,
    /// Nome desta instância no `cluster.toml` (HR-50b-2). Ausente no modo instância única.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    /// Impressão digital do **mapa** do cluster em uso, sem a instância local (HR-50b-2): é o valor
    /// que duas instâncias comparam entre si. Ausente no modo instância única.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cluster_map_fingerprint: Option<String>,
}

/// Versões de esquema lidas do disco.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct VersaoBancos {
    /// Versão do `harness.db`.
    pub harness: u32,
    /// Versão do `rag.db`.
    pub rag: u32,
}

/// Resumo de métricas (forma JSON; o `/metrics` devolve texto Prometheus).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MetricasJson {
    /// RAM residente do processo, em bytes.
    pub memory_bytes: i64,
    /// Turnos esperando na fila de entrada.
    pub queued_turns: i64,
    /// Mensagens esperando envio.
    pub queued_outbox: i64,
    /// Agentes com configuração carregada na memória.
    pub loaded_agents: i64,
    /// Segundos desde a subida.
    pub uptime_seconds: u64,
}

/// `GET /healthz`.
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
pub async fn healthz(State(estado): State<Estado>) -> Response {
    let harness = poph_store::versao_do_arquivo(estado.store.caminho_harness())
        .ok()
        .flatten()
        .unwrap_or(0);
    let rag = poph_store::versao_do_arquivo(estado.store.caminho_rag())
        .ok()
        .flatten()
        .unwrap_or(0);
    let bancos_ok = estado.store.tabelas().is_ok();
    // HR-50b-2: no modo cluster, o `/healthz` diz quem é esta instância e qual mapa ela usa, para o
    // `poph cluster check --remoto` comparar as instâncias entre si. No modo instância única, os
    // dois campos ficam ausentes e o corpo é o de sempre.
    let cluster = estado.cluster();
    let (instance_id, cluster_map_fingerprint) = if cluster.declara_cluster() {
        (
            Some(cluster.config.instance_id.trim().to_owned()),
            Some(cluster.config.map_fingerprint()),
        )
    } else {
        (None, None)
    };
    let corpo = HealthJson {
        status: if bancos_ok { "ok" } else { "degraded" }.to_owned(),
        version: crate::VERSAO.to_owned(),
        uptime_seconds: segundos_no_ar(&estado),
        databases: VersaoBancos { harness, rag },
        instance_id,
        cluster_map_fingerprint,
    };
    let status = if bancos_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(corpo)).into_response()
}

/// `GET /metrics` (texto de exposição do Prometheus).
#[utoipa::path(
    get,
    path = "/metrics",
    tag = "operacao",
    operation_id = "operacao_metrics",
    responses((status = 200, description = "Métricas em texto Prometheus", body = String))
)]
pub async fn metrics(State(estado): State<Estado>) -> Response {
    atualizar_medidores(&estado);
    let texto = estado.registro.render();
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        texto,
    )
        .into_response()
}

/// `GET /openapi.json` — o mesmo documento que está em `/admin/v1/openapi.json`.
#[utoipa::path(
    get,
    path = "/openapi.json",
    tag = "operacao",
    operation_id = "operacao_openapi",
    responses((status = 200, description = "Documento OpenAPI 3.1", body = Object))
)]
pub async fn openapi() -> Response {
    let documento = super::documento();
    let json = serde_json::to_value(&documento).unwrap_or_else(|erro| {
        tracing::error!(erro = %erro, "falha ao serializar o OpenAPI");
        serde_json::json!({ "erro": "não consegui serializar o OpenAPI" })
    });
    (StatusCode::OK, Json(json)).into_response()
}

/// Lê os números do momento e grava nos medidores publicados em `/metrics`.
fn atualizar_medidores(estado: &Estado) {
    // As contas do cache de revisões (FX-19) viram as séries `poph_fluxos_*` no momento da leitura.
    if let Some(fluxos) = estado.metricas_fluxos() {
        fluxos.sincronizar_cache(estado.cache_fluxos.metricas());
    }
    estado
        .metricas
        .process_memory_bytes
        .set(memoria_residente_bytes() as i64);
    estado
        .metricas
        .harness_db_bytes
        .set(tamanho(estado.store.caminho_harness()) as i64);
    estado
        .metricas
        .rag_db_bytes
        .set(tamanho(estado.store.caminho_rag()) as i64);

    let mut pendentes_entrada = 0i64;
    let mut pendentes_saida = 0i64;
    for org in todas_as_orgs(estado) {
        match estado.store.caixa_entrada().contar_pendentes(org) {
            Ok(quantos) => pendentes_entrada += quantos as i64,
            Err(erro) => tracing::warn!(erro = %erro, "métrica: não contei a entrada"),
        }
        match estado
            .store
            .caixa_saida()
            .contar_por_estado(org, EstadoSaida::Pendente)
        {
            Ok(quantos) => pendentes_saida += quantos as i64,
            Err(erro) => tracing::warn!(erro = %erro, "métrica: não contei a saída"),
        }
    }
    estado.metricas.turn_queue.set(pendentes_entrada);
    estado.metricas.outbox_queue.set(pendentes_saida);

    // A memória quente dos agentes ainda não é ligada no servidor (HR-13/HR-20): o medidor existe
    // e aparece zerado até a integração do motor com a API.
    estado.metricas.loaded_agents.set(0);
}

fn todas_as_orgs(estado: &Estado) -> Vec<poph_types::OrgId> {
    let mut ids = Vec::new();
    let mut apos = None;
    loop {
        let pagina = Pagina { apos, limite: 200 };
        match estado.store.orgaos().listar(pagina) {
            Ok(orgs) => {
                let quantidade = orgs.len();
                if let Some(ultima) = orgs.last() {
                    apos = Some(ultima.id.as_uuid());
                }
                ids.extend(orgs.iter().map(|org| org.id));
                if quantidade < 200 {
                    break;
                }
            }
            Err(erro) => {
                tracing::warn!(erro = %erro, "métrica: não listei as orgs");
                break;
            }
        }
    }
    ids
}

fn segundos_no_ar(estado: &Estado) -> u64 {
    SystemTime::now()
        .duration_since(estado.inicio)
        .map(|duracao| duracao.as_secs())
        .unwrap_or(0)
}

fn tamanho(caminho: &std::path::Path) -> u64 {
    std::fs::metadata(caminho)
        .map(|meta| meta.len())
        .unwrap_or(0)
}

/// RAM residente do processo. Lê `/proc/self/status` (Linux); fora dele devolve 0.
fn memoria_residente_bytes() -> u64 {
    let Ok(texto) = std::fs::read_to_string("/proc/self/status") else {
        return 0;
    };
    for linha in texto.lines() {
        if let Some(resto) = linha.strip_prefix("VmRSS:") {
            let numero: u64 = resto
                .trim()
                .trim_end_matches(" kB")
                .trim()
                .parse()
                .unwrap_or(0);
            return numero.saturating_mul(1024);
        }
    }
    0
}

/// Resumo JSON das métricas (usado pelos testes e por quem prefere JSON ao texto).
#[allow(dead_code)]
pub(crate) fn resumo(estado: &Estado) -> MetricasJson {
    MetricasJson {
        memory_bytes: memoria_residente_bytes() as i64,
        queued_turns: estado.metricas.turn_queue.value(),
        queued_outbox: estado.metricas.outbox_queue.value(),
        loaded_agents: estado.metricas.loaded_agents.value(),
        uptime_seconds: segundos_no_ar(estado),
    }
}
