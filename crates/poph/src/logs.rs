//! Logs estruturados (tracing): texto para pessoas ou JSON, uma linha por evento.
//!
//! - Nível: `RUST_LOG`, se definido; senão `log.nivel` da configuração.
//! - Saída: sempre no erro padrão (stderr), para o `poph doctor --json` continuar válido no stdout.
//! - Contexto: [`contexto_turno`] devolve um span com org, agente, conversa e turno, que vale para
//!   todos os eventos dentro dele.
//! - Regra: nenhum conteúdo de cliente nem segredo entra em log. Valores sensíveis são envolvidos
//!   em `Sensivel`, que se imprime como `[oculto]`.

use std::io;

use poph_types::{AgentId, ConversationId, Formato, LogConfig, OrgId, TurnId};
use thiserror::Error;
use tracing::{Dispatch, Span};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;

/// Variável padrão do Rust para filtrar logs. Quando definida, vence `log.nivel`.
pub const VAR_RUST_LOG: &str = "RUST_LOG";

/// Erro ao iniciar os logs.
#[derive(Debug, Error)]
pub enum ErroLog {
    /// Filtro de nível que o tracing não entende.
    #[error("nível de log inválido ({valor}): {motivo}")]
    Filtro { valor: String, motivo: String },
    /// Já existe um assinante global de log.
    #[error("não consegui iniciar os logs: {0}")]
    Inicio(String),
}

/// Instala o assinante global de log (stderr), com o nível escolhido.
pub fn iniciar(config: &LogConfig, rust_log: Option<&str>) -> Result<(), ErroLog> {
    let fonte = rust_log
        .map(str::trim)
        .filter(|valor| !valor.is_empty())
        .unwrap_or(config.nivel.as_str());
    // O runtime do modelo de embedding escreve centenas de linhas `INFO` a cada carga (HR-26h).
    // Por padrão ele fica em `warn`; quem quiser vê-lo define o alvo `ort` no `RUST_LOG`.
    let fonte = if fonte.contains("ort=") {
        fonte.to_owned()
    } else {
        format!("{fonte},ort=warn")
    };
    let filtro = filtro(&fonte)?;
    let assinante = construir(config.formato, filtro, io::stderr);
    tracing::dispatcher::set_global_default(assinante)
        .map_err(|erro| ErroLog::Inicio(erro.to_string()))
}

/// Monta o assinante com formato, filtro e destino dados. Usado no início e nos testes.
pub fn construir<W>(formato: Formato, filtro: EnvFilter, escritor: W) -> Dispatch
where
    W: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    match formato {
        Formato::Legivel => Dispatch::new(
            tracing_subscriber::fmt()
                .with_env_filter(filtro)
                .with_writer(escritor)
                .with_ansi(false)
                .finish(),
        ),
        Formato::Json => Dispatch::new(
            tracing_subscriber::fmt()
                .json()
                .with_env_filter(filtro)
                .with_writer(escritor)
                .finish(),
        ),
    }
}

/// Span de contexto de um turno. Todo evento dentro dele sai com org, agente, conversa e turno.
// Ainda sem uso no binário: o turno que o abre chega na HR-13. Os testes deste módulo já o usam.
#[allow(dead_code)]
pub fn contexto_turno(
    org: OrgId,
    agente: AgentId,
    conversa: ConversationId,
    turno: TurnId,
) -> Span {
    tracing::info_span!(
        "turno",
        org = %org,
        agente = %agente,
        conversa = %conversa,
        turno = %turno
    )
}

fn filtro(nivel: &str) -> Result<EnvFilter, ErroLog> {
    EnvFilter::try_new(nivel).map_err(|erro| ErroLog::Filtro {
        valor: nivel.to_owned(),
        motivo: erro.to_string(),
    })
}

#[cfg(test)]
mod testes {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use poph_types::Sensivel;

    use super::*;

    /// Guarda a saída do log em memória, para o teste ler.
    #[derive(Clone, Default)]
    struct Captura(Arc<Mutex<Vec<u8>>>);

    impl Captura {
        fn texto(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().expect("trava")).into_owned()
        }
    }

    impl Write for Captura {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("trava").extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn captura_com(formato: Formato, nivel: &str) -> (Dispatch, Captura) {
        let captura = Captura::default();
        let escrita = captura.clone();
        let dispatch = construir(formato, filtro(nivel).expect("filtro"), move || {
            escrita.clone()
        });
        (dispatch, captura)
    }

    #[test]
    fn span_de_turno_leva_org_agente_conversa_e_turno_no_json() {
        let (dispatch, captura) = captura_com(Formato::Json, "info");
        let org = OrgId::generate();
        let agente = AgentId::generate();
        let conversa = ConversationId::generate();
        let turno = TurnId::generate();
        tracing::dispatcher::with_default(&dispatch, || {
            let span = contexto_turno(org, agente, conversa, turno);
            let _dentro = span.enter();
            tracing::info!("turno iniciado");
        });
        let saida = captura.texto();
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
        for linha in saida.lines() {
            serde_json::from_str::<serde_json::Value>(linha)
                .unwrap_or_else(|erro| panic!("linha de log não é JSON válido ({erro}): {linha}"));
        }
    }

    #[test]
    fn formato_legivel_tambem_mostra_o_contexto() {
        let (dispatch, captura) = captura_com(Formato::Legivel, "info");
        let org = OrgId::generate();
        tracing::dispatcher::with_default(&dispatch, || {
            let span = contexto_turno(
                org,
                AgentId::generate(),
                ConversationId::generate(),
                TurnId::generate(),
            );
            let _dentro = span.enter();
            tracing::info!("evento de teste");
        });
        assert!(captura.texto().contains(&org.to_string()));
    }

    #[test]
    fn valor_sensivel_nunca_aparece_no_log() {
        let (dispatch, captura) = captura_com(Formato::Json, "trace");
        tracing::dispatcher::with_default(&dispatch, || {
            let conteudo = Sensivel::new("texto-privado-do-cliente-42".to_owned());
            tracing::info!(conteudo = %conteudo, "mensagem recebida");
            tracing::debug!(conteudo = ?conteudo, "depuração");
        });
        let saida = captura.texto();
        assert!(saida.contains("mensagem recebida"));
        assert!(
            !saida.contains("texto-privado-do-cliente-42"),
            "vazou: {saida}"
        );
    }

    #[test]
    fn nivel_filtra_eventos_abaixo_dele() {
        let (dispatch, captura) = captura_com(Formato::Legivel, "warn");
        tracing::dispatcher::with_default(&dispatch, || {
            tracing::info!("nao deve sair");
            tracing::warn!("deve sair");
        });
        let saida = captura.texto();
        assert!(!saida.contains("nao deve sair"));
        assert!(saida.contains("deve sair"));
    }

    #[test]
    fn nivel_invalido_e_recusado_com_o_valor() {
        match filtro("poph=supremo") {
            Err(ErroLog::Filtro { valor, .. }) => assert_eq!(valor, "poph=supremo"),
            outro => panic!("esperado erro de filtro, veio {outro:?}"),
        }
    }
}
