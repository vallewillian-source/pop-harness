//! Servidor do POP Harness: API administrativa (HR-20), autenticação por token, OpenAPI e
//! métricas. O chat de teste com SSE entrou na HR-22; webhooks, roteamento entre instâncias
//! e plugins seguem nas tarefas seguintes.

pub mod api;
pub mod auth;
pub mod cifra;
pub mod cluster;
pub mod console;
pub mod encaminhamento;
pub mod entrada_turno;
pub mod erro;
pub mod estado;
pub mod evolution;
pub mod fluxos_metricas;
pub mod manifestos;
pub mod matrix_espelho;
pub mod matrix_estado;
pub mod matrix_expiracao;
pub mod matrix_falhas_aviso;
pub mod matrix_inicio;
pub mod matrix_metricas;
pub mod matrix_resposta;
pub mod saida_whatsapp;

use std::future::Future;

use axum::Router;
use tokio::net::TcpListener;

pub use api::matrix_as::{
    EntregaDeEventos, ErroEntrega, FuturoDaEntrega, ReceptorAs, TETO_CORPO, TETO_PADRAO_TRANSACOES,
};
pub use cifra::ChaveDeCifra;
pub use erro::{ApiError, RespostaErro};
pub use estado::Estado;
pub use manifestos::CatalogoManifestos;
pub use matrix_resposta::{PortaAtendimento, PortaDoHarness, RespostaDoAtendente};

/// Versão do servidor (a mesma do binário `poph`).
pub const VERSAO: &str = env!("CARGO_PKG_VERSION");

/// Monta o `Router` completo (API, saúde, métricas e OpenAPI).
pub fn router(estado: Estado) -> Router {
    api::router(estado)
}

/// Documento OpenAPI da API administrativa.
pub fn openapi() -> utoipa::openapi::OpenApi {
    api::documento()
}

/// Sobe o servidor num `TcpListener` já aberto (é o que os testes usam com porta efêmera).
///
/// `desligamento` é aguardado para encerrar com calma: o servidor para de aceitar conexões novas
/// e espera as em andamento.
pub async fn servir_em<F>(
    listener: TcpListener,
    mut estado: Estado,
    desligamento: F,
) -> std::io::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    // HR-57: turno que ficou em andamento quando o processo anterior morreu vira falha registrada,
    // antes de qualquer atendimento — o histórico não pode dizer "em andamento" para sempre.
    entrada_turno::recuperar_turnos_interrompidos(&estado);
    // Saída do WhatsApp (HR-26c): o laço só sobe quando há instância configurada, e para junto
    // com o servidor.
    let saida = saida_whatsapp::iniciar(&estado);
    // Turno no servidor (HR-26h): só sobe com o fluxo ligado na raiz de composição.
    let turno = entrada_turno::iniciar(&estado);
    // Módulo Matrix (MX-20): desligado por configuração, não sobe nada e as rotas do Synapse
    // respondem `404`. Ligado, põe os laços no ar e injeta o receptor do Application Service. Falha
    // de configuração ou de conexão não impede o servidor de atender — o módulo fica degradado.
    let matrix = matrix_inicio::iniciar(&mut estado);
    let resultado = axum::serve(listener, router(estado))
        .with_graceful_shutdown(desligamento)
        .await;
    // O módulo Matrix para antes dos outros: os laços terminam o que estão fazendo (o cursor só
    // anda depois do sucesso), então nada pendente se perde.
    if let Some(matrix) = matrix {
        matrix.parar().await;
    }
    if let Some(saida) = saida {
        saida.abort();
    }
    if let Some(turno) = turno {
        turno.abort();
    }
    resultado
}

/// Abre a porta do endereço e sobe o servidor até o desligamento.
pub async fn servir<F>(
    endereco: std::net::SocketAddr,
    estado: Estado,
    desligamento: F,
) -> std::io::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let listener = TcpListener::bind(endereco).await?;
    servir_em(listener, estado, desligamento).await
}
