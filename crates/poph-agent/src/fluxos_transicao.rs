//! Ponte entre o motor de fluxos (`poph-fluxos`, puro) e a gravação atômica (`poph-store`, FX-20).
//!
//! O motor diz **o que** aconteceu (estado novo, eventos, efeitos, pedido de encerramento). O
//! repositório grava no vocabulário das colunas, sem importar o motor (regra de camadas do
//! `AGENTS.md`). Quem traduz de um para o outro é este módulo, e só ele: o turno (FX-23) e a API de
//! ações humanas (FX-30) usam estas funções, para não existirem dois formatos.
//!
//! Mora aqui — e não no `poph-server` — porque **o turno também precisa dele**: `poph-agent` é
//! dependência do servidor, então os dois lados compartilham uma tradução só. O servidor reexporta
//! estes itens em `crate::fluxos_transicao` e em `crate::fluxos_efeitos::efeito_da_transicao`, e os
//! chamadores existentes não mudam.
//!
//! Nenhuma decisão do motor é refeita aqui: são conversões de vocabulário.

use poph_fluxos::EfeitoProduzido;
use poph_fluxos::motor::{Estado as EstadoMotor, Resultado as ResultadoMotor};
use poph_store::{ExecutionState, FlowTransition, TransitionEffect, TransitionEvent};

/// O estado do motor no vocabulário das colunas de `fluxo_execucoes` (e da etapa de entrada).
///
/// O nó vai pelo **id**: é o que a coluna `no_atual` guarda. `chaves` não é persistido (a
/// idempotência é o índice único de `ficha_eventos`, FX-20).
pub fn estado_para_banco(estado: &EstadoMotor) -> ExecutionState {
    ExecutionState {
        status: estado.status,
        no_atual: estado.no_atual.id().to_owned(),
        dados_json: serde_json::Value::Object(estado.dados.clone()).to_string(),
        acting_actors: estado.atores_que_agiram.clone(),
        result: estado.resultado.clone(),
        reason: estado.motivo.clone(),
    }
}

/// O desfecho de uma passagem do motor (ação, abandono) pronto para a operação atômica do store.
///
/// `repetida` vem do motor, que não guarda as chaves: quem confere repetição de verdade é o store
/// (pela marca em `ficha_eventos`), e a API checa antes de chamar o motor (ver `api::fichas`).
pub fn transicao_do_motor(resultado: &ResultadoMotor) -> FlowTransition {
    FlowTransition {
        new_state: estado_para_banco(&resultado.novo_estado),
        events: resultado.eventos.iter().map(evento_para_banco).collect(),
        effects: resultado.efeitos.iter().map(efeito_da_transicao).collect(),
        closes_service: resultado.encerra_atendimento,
        repeated: resultado.repetida,
        // FX-40: o motivo da falha é texto no espelho; o motor diz aqui se foi regra que não avaliou.
        rule_error: resultado.erro_de_regra,
    }
}

/// Um evento do motor no vocabulário de `ficha_eventos`.
fn evento_para_banco(evento: &poph_fluxos::motor::Evento) -> TransitionEvent {
    TransitionEvent {
        tipo: evento.tipo,
        ator_tipo: evento.ator,
        payload_json: evento.payload.to_string(),
    }
}

/// Traduz o efeito do motor no que a fila grava (FX-20) — o **único** lugar do formato.
///
/// O adaptador do turno e a API montam [`EfeitoDaTransicao`] com esta função, e o entregador da
/// fila (`poph_server::fluxos_efeitos`) lê o mesmo formato: não existem dois jeitos de escrever o
/// payload.
pub fn efeito_da_transicao(produzido: &EfeitoProduzido) -> TransitionEffect {
    let payload = match &produzido.efeito {
        poph_fluxos::ir::Efeito::MensagemAoCliente { texto } => {
            serde_json::json!({ "texto": texto })
        }
        poph_fluxos::ir::Efeito::NotificarWhatsapp { para, mensagem } => {
            serde_json::json!({ "para": para, "mensagem": mensagem })
        }
        poph_fluxos::ir::Efeito::Encaminhar { departamento } => {
            serde_json::json!({ "departamento": departamento })
        }
    };
    TransitionEffect {
        key: produzido.chave.clone(),
        tipo: produzido.tipo().to_owned(),
        payload_json: payload.to_string(),
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    /// O formato do `payload_json` é contrato com a fila de efeitos (FX-21): os três tipos, um por
    /// um, sem chute de nome de campo.
    #[test]
    fn payload_de_cada_tipo_de_efeito() {
        let produzido = |efeito: poph_fluxos::ir::Efeito| EfeitoProduzido {
            chave: "etapa:coleta/0".to_owned(),
            no: "etapa:coleta".to_owned(),
            indice: 0,
            efeito,
        };
        let mensagem =
            efeito_da_transicao(&produzido(poph_fluxos::ir::Efeito::MensagemAoCliente {
                texto: "Já vou verificar.".to_owned(),
            }));
        assert_eq!(mensagem.tipo, "mensagem_ao_cliente");
        assert_eq!(mensagem.key, "etapa:coleta/0");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&mensagem.payload_json).expect("JSON"),
            serde_json::json!({ "texto": "Já vou verificar." })
        );

        let notificacao =
            efeito_da_transicao(&produzido(poph_fluxos::ir::Efeito::NotificarWhatsapp {
                para: "5511999990000".to_owned(),
                mensagem: "Cliente pediu visita.".to_owned(),
            }));
        assert_eq!(notificacao.tipo, "notificar_whatsapp");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&notificacao.payload_json).expect("JSON"),
            serde_json::json!({ "para": "5511999990000", "mensagem": "Cliente pediu visita." })
        );

        let encaminhar = efeito_da_transicao(&produzido(poph_fluxos::ir::Efeito::Encaminhar {
            departamento: "Comercial".to_owned(),
        }));
        assert_eq!(encaminhar.tipo, "encaminhar");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&encaminhar.payload_json).expect("JSON"),
            serde_json::json!({ "departamento": "Comercial" })
        );
    }
}
