pub mod api;
pub mod auth;
pub mod cipher;
pub mod cluster;
pub mod console;
pub mod encaminhamento;
pub mod entrada_turno;
pub mod error;
pub mod estado;
pub mod evolution;
pub mod fichas_servico;
pub mod fluxos_efeitos;
pub mod fluxos_metricas;
pub mod fluxos_servico;
pub mod fluxos_transicao;
pub mod manifestos;
pub mod matrix_beginning;
pub mod matrix_expiry;
pub mod matrix_failure_notice;
pub mod matrix_metrics;
pub mod matrix_mirror;
pub mod matrix_response;
pub mod matrix_state;
pub mod saida_whatsapp;
mod site_chat;

use std::future::Future;

use axum::Router;
use tokio::net::TcpListener;

pub use api::matrix_as::{
    AsReceiver, CAP_BODY, CAP_DEFAULT_TRANSACTION, DeliveryError, DeliveryFuture, EventDelivery,
};
pub use cipher::CipherKey;
pub use error::{ApiError, ErrorResponse};
pub use estado::Estado;
pub use manifestos::CatalogoManifestos;
pub use matrix_response::{AttendantResponse, HarnessResponsePort, ResponsePort};
pub(crate) use saida_whatsapp::MOTIVO_TENTATIVAS_ESGOTADAS as WHATSAPP_ATTEMPTS_EXHAUSTED;

pub(crate) fn last_response_error(failure: &poph_store::HumanResponseFailure) -> Option<&str> {
    failure.ultimo_erro.as_deref()
}

pub const VERSAO: &str = env!("CARGO_PKG_VERSION");

pub fn router(estado: Estado) -> Router {
    api::router(estado)
}

pub fn openapi() -> utoipa::openapi::OpenApi {
    api::document()
}

pub async fn serve_at<F>(
    listener: TcpListener,
    mut estado: Estado,
    desligamento: F,
) -> std::io::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    entrada_turno::recuperar_turnos_interrompidos(&estado);
    site_chat::recover(&estado).map_err(std::io::Error::other)?;
    let saida = saida_whatsapp::iniciar(&estado);
    let effects = fluxos_efeitos::iniciar(&estado);
    let turno = entrada_turno::iniciar(&estado);
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
    estado: Estado,
    desligamento: F,
) -> std::io::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let listener = TcpListener::bind(endereco).await?;
    serve_at(listener, estado, desligamento).await
}
