pub mod api;
pub mod auth;
pub mod case_service;
pub mod cipher;
pub mod cluster;
pub mod console;
pub mod error;
pub mod evolution;
pub mod flow_effects;
pub mod flow_metrics;
pub mod flow_service;
pub mod flow_transition;
pub mod forwarding;
pub mod http_effects;
pub mod manifests;
pub mod matrix_beginning;
pub mod matrix_expiry;
pub mod matrix_failure_notice;
pub mod matrix_metrics;
pub mod matrix_mirror;
pub mod matrix_response;
pub mod matrix_state;
mod site_chat;
pub mod state;
pub mod turn_input;
pub mod whatsapp_output;

use std::future::Future;

use axum::Router;
use tokio::net::TcpListener;

pub use api::matrix_as::{
    AsReceiver, CAP_BODY, CAP_DEFAULT_TRANSACTION, DeliveryError, DeliveryFuture, EventDelivery,
};
pub use cipher::CipherKey;
pub use error::{ApiError, ErrorResponse};
pub use manifests::ManifestCatalog;
pub use matrix_response::{AttendantResponse, HarnessResponsePort, ResponsePort};
pub use state::ServerState;
pub(crate) use whatsapp_output::RETRY_LIMIT_REASON as WHATSAPP_ATTEMPTS_EXHAUSTED;

pub(crate) fn last_response_error(failure: &poph_store::HumanResponseFailure) -> Option<&str> {
    failure.ultimo_erro.as_deref()
}

pub const VERSAO: &str = env!("CARGO_PKG_VERSION");

pub fn router(estado: ServerState) -> Router {
    api::router(estado)
}

pub fn openapi() -> utoipa::openapi::OpenApi {
    api::document()
}

pub async fn serve_at<F>(
    listener: TcpListener,
    mut estado: ServerState,
    desligamento: F,
) -> std::io::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    turn_input::recover_interrupted_turns(&estado);
    site_chat::recover(&estado).map_err(std::io::Error::other)?;
    let saida = whatsapp_output::start(&estado);
    let effects = flow_effects::start(&estado);
    let turno = turn_input::start(&estado);
    let matrix = matrix_beginning::start(&mut estado);
    let resultado = axum::serve(listener, router(estado))
        .with_graceful_shutdown(desligamento)
        .await;
    if let Some(matrix) = matrix {
        matrix.stop().await;
    }
    if let Some(saida) = saida {
        saida.abort();
    }
    if let Some(turno) = turno {
        turno.abort();
    }
    if let Some(effects) = effects {
        effects.abort();
    }
    resultado
}

pub async fn serve<F>(
    endereco: std::net::SocketAddr,
    estado: ServerState,
    desligamento: F,
) -> std::io::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let listener = TcpListener::bind(endereco).await?;
    serve_at(listener, estado, desligamento).await
}
