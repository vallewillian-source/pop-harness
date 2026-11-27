use std::io;

use poph_types::{AgentId, ConversationId, Format, LogConfig, OrgId, TurnId};
use thiserror::Error;
use tracing::{Dispatch, Span};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;

pub const VAR_RUST_LOG: &str = "RUST_LOG";

#[derive(Debug, Error)]
pub enum LogError {
    #[error("nível de log inválido ({value}): {motivo}")]
    Filter { value: String, motivo: String },
    #[error("não consegui iniciar os logs: {0}")]
    Start(String),
}

pub fn start(config: &LogConfig, rust_log: Option<&str>) -> Result<(), LogError> {
    let fonte = rust_log
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(config.level.as_str());
    let fonte = if fonte.contains("ort=") {
        fonte.to_owned()
    } else {
        format!("{fonte},ort=warn")
    };
    let filter = parse_filter(&fonte)?;
    let assinante = build(config.format, filter, io::stderr);
    tracing::dispatcher::set_global_default(assinante)
        .map_err(|error| LogError::Start(error.to_string()))
}

pub fn build<W>(formato: Format, filter: EnvFilter, writer: W) -> Dispatch
where
    W: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    match formato {
        Format::Legible => Dispatch::new(
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_writer(writer)
                .with_ansi(false)
                .finish(),
        ),
        Format::Json => Dispatch::new(
            tracing_subscriber::fmt()
                .json()
                .with_env_filter(filter)
                .with_writer(writer)
                .finish(),
        ),
    }
}

#[allow(dead_code)]
pub fn turn_context(org: OrgId, agente: AgentId, conversa: ConversationId, turno: TurnId) -> Span {
    tracing::info_span!(
        "turno",
        org = %org,
        agente = %agente,
        conversa = %conversa,
        turno = %turno
    )
}

fn parse_filter(nivel: &str) -> Result<EnvFilter, LogError> {
    EnvFilter::try_new(nivel).map_err(|error| LogError::Filter {
        value: nivel.to_owned(),
        motivo: error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use poph_types::Sensitive;

    use super::*;

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Capture {
        fn texto(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().expect("trava")).into_owned()
        }
    }

    impl Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("trava").extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn capture_with(formato: Format, nivel: &str) -> (Dispatch, Capture) {
        let capture = Capture::default();
        let handle = capture.clone();
        let dispatch = build(formato, parse_filter(nivel).expect("filtro"), move || {
            handle.clone()
        });
        (dispatch, capture)
    }

    #[test]
    fn turn_span_carries_org_agent_conversation_and_turn_in_json() {
        let (dispatch, capture) = capture_with(Format::Json, "info");
        let org = OrgId::generate();
        let agente = AgentId::generate();
        let conversa = ConversationId::generate();
        let turno = TurnId::generate();
        tracing::dispatcher::with_default(&dispatch, || {
            let span = turn_context(org, agente, conversa, turno);
            let _inside = span.enter();
            tracing::info!("turno iniciado");
        });
        let saida = capture.texto();
        assert!(saida.contains(&org.to_string()), "faltou org: {saida}");
        assert!(
            saida.contains(&agente.to_string()),
            "faltou agente: {saida}"
        );
        assert!(
            saida.contains(&conversa.to_string()),
            "faltou conversa: {saida}"
        );
        assert!(saida.contains(&turno.to_string()), "faltou turno: {saida}");
        for line in saida.lines() {
            serde_json::from_str::<serde_json::Value>(line)
                .unwrap_or_else(|error| panic!("linha de log não é JSON válido ({error}): {line}"));
        }
    }

    #[test]
    fn readable_format_also_shows_context() {
        let (dispatch, capture) = capture_with(Format::Legible, "info");
        let org = OrgId::generate();
        tracing::dispatcher::with_default(&dispatch, || {
            let span = turn_context(
                org,
                AgentId::generate(),
                ConversationId::generate(),
                TurnId::generate(),
            );
            let _inside = span.enter();
            tracing::info!("evento de teste");
        });
        assert!(capture.texto().contains(&org.to_string()));
    }

    #[test]
    fn sensitive_value_never_appears_in_log() {
        let (dispatch, capture) = capture_with(Format::Json, "trace");
        tracing::dispatcher::with_default(&dispatch, || {
            let content = Sensitive::new("texto-privado-do-cliente-42".to_owned());
            tracing::info!(content = %content, "mensagem recebida");
            tracing::debug!(content = ?content, "depuração");
        });
        let saida = capture.texto();
        assert!(saida.contains("mensagem recebida"));
        assert!(
            !saida.contains("texto-privado-do-cliente-42"),
            "vazou: {saida}"
        );
    }

    #[test]
    fn level_filters_events_below_it() {
        let (dispatch, capture) = capture_with(Format::Legible, "warn");
        tracing::dispatcher::with_default(&dispatch, || {
            tracing::info!("nao deve sair");
            tracing::warn!("deve sair");
        });
        let saida = capture.texto();
        assert!(!saida.contains("nao deve sair"));
        assert!(saida.contains("deve sair"));
    }

    #[test]
    fn invalid_level_is_refused_with_value() {
        match parse_filter("poph=supremo") {
            Err(LogError::Filter { value, .. }) => assert_eq!(value, "poph=supremo"),
            other => panic!("esperado erro de filtro, veio {other:?}"),
        }
    }
}
