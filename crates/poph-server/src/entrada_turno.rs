//! Turno no `poph serve` (HR-26h): leitura da fila de mensagens recebidas e despacho para o turno.
//!
//! Roda como um laço dentro do `poph serve` (ver [`iniciar`]), só quando o turno está ligado
//! (ver [`crate::estado::Estado::com_turno`]). A cada intervalo, para cada org:
//!
//! - **Despacha só a entrada.** O item da fila de entrada vai para o ator da conversa, que roda o
//!   turno e grava a resposta na fila de saída. A entrega dessa resposta é do laço de saída
//!   ([`crate::saida_whatsapp`]). Por isso não se usa `Fluxo::ciclo`: ele também envia a saída pelo
//!   transporte recebido, e o do servidor é o do WhatsApp.
//! - **Retomada na subida.** A primeira passagem já pega o que ficou pendente enquanto o servidor
//!   estava parado. Item que estava em voo quando o processo caiu continua na fila, e o turno decide
//!   pelo histórico se a mensagem já foi respondida (não responde duas vezes).
//! - **Um turno por conversa.** Quem garante é o ator da conversa (uma fila por conversa); aqui só
//!   se reserva o item para não entregá-lo duas vezes.

use std::time::Duration;

use poph_agent::entrada_saida::Fluxo;
use poph_agent::turno::TransporteConsole;
use tokio::task::JoinHandle;

use crate::estado::Estado;

/// Motivo gravado no turno que ficou aberto quando o processo anterior morreu (HR-57).
///
/// Curto e sem dado do cliente: vai para a coluna `error` e aparece no histórico do turno.
pub const MOTIVO_INTERROMPIDO: &str =
    "interrompido: o processo caiu no meio do atendimento; a mensagem foi mantida";

/// Sobe o laço de despacho quando o turno está ligado no estado. Sem turno, não sobe nada.
pub fn iniciar(estado: &Estado) -> Option<JoinHandle<()>> {
    let fluxo = estado.turno.clone()?;
    let intervalo: Duration = estado.intervalo_turno;
    tracing::info!(
        target: "poph_server::turno",
        intervalo_ms = u64::try_from(intervalo.as_millis()).unwrap_or(u64::MAX),
        "turno ligado: as mensagens recebidas são lidas e respondidas"
    );
    Some(tokio::spawn(async move {
        // HR-57: antes de despachar, confirma o que ficou na fila de saída do console de um
        // processo anterior. A resposta já foi mostrada ao operador pelo stream; sem isto, o item
        // ficaria preso em `sending` até o próximo turno daquela conversa.
        confirmar_saidas_do_console(&fluxo).await;
        loop {
            despachar_uma_vez(&fluxo);
            tokio::time::sleep(intervalo).await;
        }
    }))
}

/// Fecha como falho o turno que ficou `running` de um processo anterior (HR-57).
///
/// Roda uma vez na subida do servidor, para cada org, antes de qualquer atendimento: o registro do
/// turno não pode dizer "em andamento" para sempre depois de uma queda. Turnos **começados agora**
/// (nesta instância) não são tocados, porque só entram os anteriores ao instante da subida.
///
/// Devolve quantos turnos foram fechados.
pub fn recuperar_turnos_interrompidos(estado: &Estado) -> u64 {
    let antes_de = estado
        .inicio
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duracao| i64::try_from(duracao.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(i64::MAX);
    let mut fechados = 0u64;
    let mut orgs = 0u64;
    for org in orgs_conhecidas(estado) {
        orgs += 1;
        match estado
            .store
            .turns()
            .falhar_orfaos(org, MOTIVO_INTERROMPIDO, antes_de)
        {
            Ok(quantos) => fechados += quantos,
            Err(erro) => tracing::warn!(
                target: "poph_server::turno",
                org = %org,
                motivo = %erro,
                "não consegui fechar os turnos interrompidos desta org; tenta na próxima subida"
            ),
        }
    }
    if fechados > 0 {
        tracing::warn!(
            target: "poph_server::turno",
            turnos = fechados,
            "turnos interrompidos por queda anterior fechados como falha"
        );
    } else {
        tracing::debug!(
            target: "poph_server::turno",
            orgs,
            "nenhum turno interrompido por queda anterior"
        );
    }
    fechados
}

/// Confirma a saída que ficou na fila de conversas do console (HR-57).
///
/// Não imprime nem entrega nada: o texto já foi mostrado ao operador pelo stream. Só tira o item de
/// `pending`/`sending`, para ele não ficar preso para sempre depois de uma queda.
async fn confirmar_saidas_do_console(fluxo: &Fluxo) {
    let orgs = match fluxo.orgs() {
        Ok(orgs) => orgs,
        Err(erro) => {
            tracing::warn!(
                target: "poph_server::turno",
                motivo = %erro,
                "não consegui listar as orgs para confirmar a saída do console; segue o laço"
            );
            return;
        }
    };
    let transporte = TransporteConsole::silencioso();
    for org in orgs {
        match fluxo.enviar_pendentes_do_console(org, &transporte).await {
            Ok(relato) if relato.enviados > 0 => tracing::info!(
                target: "poph_server::turno",
                org = %org,
                itens = relato.enviados,
                "saída do console confirmada na subida"
            ),
            Ok(_) => {}
            Err(erro) => tracing::warn!(
                target: "poph_server::turno",
                org = %org,
                motivo = %erro,
                "não consegui confirmar a saída do console na subida; tenta no próximo turno"
            ),
        }
    }
}

/// Ids das orgs do banco, paginando pelo id (v7 é ordenável por tempo).
fn orgs_conhecidas(estado: &Estado) -> Vec<poph_types::OrgId> {
    let mut orgs = Vec::new();
    let mut apos = None;
    loop {
        match estado.store.orgs().listar(poph_store::Page {
            after: apos,
            limit: 200,
        }) {
            Ok(pagina) => {
                let ultima = pagina.len() < 200;
                apos = pagina.last().map(|org| org.id.as_uuid());
                orgs.extend(pagina.into_iter().map(|org| org.id));
                if ultima {
                    break;
                }
            }
            Err(erro) => {
                tracing::warn!(
                    target: "poph_server::turno",
                    motivo = %erro,
                    "não consegui listar as orgs para a recuperação de turnos"
                );
                break;
            }
        }
    }
    orgs
}

/// Uma passagem: despacha a entrada pendente de cada org e atualiza os medidores das filas.
fn despachar_uma_vez(fluxo: &Fluxo) {
    let orgs = match fluxo.orgs() {
        Ok(orgs) => orgs,
        Err(erro) => {
            tracing::warn!(
                target: "poph_server::turno",
                motivo = %erro,
                "não consegui listar as orgs para ler a fila; tenta no próximo ciclo"
            );
            return;
        }
    };
    for org in orgs {
        if let Err(erro) = fluxo.despachar(org) {
            tracing::warn!(
                target: "poph_server::turno",
                org = %org,
                motivo = %erro,
                "não consegui ler a fila de mensagens recebidas; tenta no próximo ciclo"
            );
        }
    }
    if let Err(erro) = fluxo.atualizar_medidores() {
        tracing::warn!(
            target: "poph_server::turno",
            motivo = %erro,
            "não consegui atualizar as medidas das filas"
        );
    }
}
