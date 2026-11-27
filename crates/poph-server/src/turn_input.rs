use std::time::Duration;

use poph_agent::entry_output::Fluxo;
use poph_agent::turno::TransportConsole;
use tokio::task::JoinHandle;

use crate::state::ServerState;

pub const INTERRUPTED_REASON: &str =
    "interrompido: o processo caiu no meio do atendimento; a mensagem foi mantida";

pub fn start(state: &ServerState) -> Option<JoinHandle<()>> {
    let flow = state.turn.clone()?;
    let interval: Duration = state.turn_interval;
    tracing::info!(
        target: "poph_server::turno",
        interval_ms = u64::try_from(interval.as_millis()).unwrap_or(u64::MAX),
        "turno ligado: as mensagens recebidas são lidas e respondidas"
    );
    Some(tokio::spawn(async move {
        acknowledge_console_output(&flow).await;
        loop {
            dispatch_once(&flow);
            tokio::time::sleep(interval).await;
        }
    }))
}

pub fn recover_interrupted_turns(state: &ServerState) -> u64 {
    let before = state
        .started_at
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(i64::MAX);
    let mut closed = 0u64;
    let mut orgs = 0u64;
    for org in known_orgs(state) {
        orgs += 1;
        match state
            .store
            .turns()
            .fail_orphans(org, INTERRUPTED_REASON, before)
        {
            Ok(count) => closed += count,
            Err(error) => tracing::warn!(
                target: "poph_server::turno",
                org = %org,
                reason = %error,
                "não consegui fechar os turnos interrompidos desta org; tenta na próxima subida"
            ),
        }
    }
    if closed > 0 {
        tracing::warn!(
            target: "poph_server::turno",
            closed_turns = closed,
            "turnos interrompidos por queda anterior fechados como falha"
        );
    } else {
        tracing::debug!(
            target: "poph_server::turno",
            orgs,
            "nenhum turno interrompido por queda anterior"
        );
    }
    closed
}

async fn acknowledge_console_output(flow: &Fluxo) {
    let orgs = match flow.orgs() {
        Ok(orgs) => orgs,
        Err(error) => {
            tracing::warn!(
                target: "poph_server::turno",
                reason = %error,
                "não consegui listar as orgs para confirmar a saída do console; segue o laço"
            );
            return;
        }
    };
    let transport = TransportConsole::silencioso();
    for org in orgs {
        match flow.send_pending_console(org, &transport).await {
            Ok(report) if report.sent > 0 => tracing::info!(
                target: "poph_server::turno",
                org = %org,
                items = report.sent,
                "saída do console confirmada na subida"
            ),
            Ok(_) => {}
            Err(error) => tracing::warn!(
                target: "poph_server::turno",
                org = %org,
                reason = %error,
                "não consegui confirmar a saída do console na subida; tenta no próximo turno"
            ),
        }
    }
}

fn known_orgs(state: &ServerState) -> Vec<poph_types::OrgId> {
    let mut orgs = Vec::new();
    let mut after = None;
    loop {
        match state
            .store
            .orgs()
            .list(poph_store::Page { after, limit: 200 })
        {
            Ok(page) => {
                let last = page.len() < 200;
                after = page.last().map(|org| org.id.as_uuid());
                orgs.extend(page.into_iter().map(|org| org.id));
                if last {
                    break;
                }
            }
            Err(error) => {
                tracing::warn!(
                    target: "poph_server::turno",
                    reason = %error,
                    "não consegui listar as orgs para a recuperação de turnos"
                );
                break;
            }
        }
    }
    orgs
}

fn dispatch_once(flow: &Fluxo) {
    let orgs = match flow.orgs() {
        Ok(orgs) => orgs,
        Err(error) => {
            tracing::warn!(
                target: "poph_server::turno",
                reason = %error,
                "não consegui listar as orgs para ler a fila; tenta no próximo ciclo"
            );
            return;
        }
    };
    for org in orgs {
        if let Err(error) = flow.dispatch(org) {
            tracing::warn!(
                target: "poph_server::turno",
                org = %org,
                reason = %error,
                "não consegui ler a fila de mensagens recebidas; tenta no próximo ciclo"
            );
        }
    }
    if let Err(error) = flow.update_medidores() {
        tracing::warn!(
            target: "poph_server::turno",
            reason = %error,
            "não consegui atualizar as medidas das filas"
        );
    }
}
