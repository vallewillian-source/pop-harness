use poph_fluxos::ProducedEffect;
use poph_fluxos::motor::{Estado as StateEngine, Resultado as ResultEngine};
use poph_store::{ExecutionState, FlowTransition, TransitionEffect, TransitionEvent};

pub fn state_for_database(estado: &StateEngine) -> ExecutionState {
    ExecutionState {
        status: estado.status,
        no_atual: estado.no_atual.id().to_owned(),
        dados_json: serde_json::Value::Object(estado.dados.clone()).to_string(),
        acting_actors: estado.actors_who_acted.clone(),
        result: estado.resultado.clone(),
        reason: estado.motivo.clone(),
    }
}

pub fn transition_engine(resultado: &ResultEngine) -> FlowTransition {
    FlowTransition {
        new_state: state_for_database(&resultado.new_state),
        events: resultado.events.iter().map(event_for_database).collect(),
        effects: resultado.effects.iter().map(effect_of_transition).collect(),
        closes_service: resultado.ends_service,
        repeated: resultado.repeated,
        rule_error: resultado.rule_error,
    }
}

fn event_for_database(evento: &poph_fluxos::motor::Evento) -> TransitionEvent {
    TransitionEvent {
        tipo: evento.tipo,
        ator_tipo: evento.actor,
        payload_json: evento.payload.to_string(),
    }
}

pub fn effect_of_transition(produced: &ProducedEffect) -> TransitionEffect {
    let payload = match &produced.effect {
        poph_fluxos::ir::Effect::MessageToCustomer { texto } => {
            serde_json::json!({ "texto": texto })
        }
        poph_fluxos::ir::Effect::NotifyWhatsApp { para, mensagem } => {
            serde_json::json!({ "para": para, "mensagem": mensagem })
        }
        poph_fluxos::ir::Effect::Encaminhar { departamento } => {
            serde_json::json!({ "departamento": departamento })
        }
    };
    TransitionEffect {
        key: produced.chave.clone(),
        tipo: produced.tipo().to_owned(),
        payload_json: payload.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_of_each_kind_of_effect() {
        let produced = |effect: poph_fluxos::ir::Effect| ProducedEffect {
            chave: "etapa:coleta/0".to_owned(),
            no: "etapa:coleta".to_owned(),
            index: 0,
            effect,
        };
        let message_effect =
            effect_of_transition(&produced(poph_fluxos::ir::Effect::MessageToCustomer {
                texto: "Já vou verificar.".to_owned(),
            }));
        assert_eq!(message_effect.tipo, "mensagem_ao_cliente");
        assert_eq!(message_effect.key, "etapa:coleta/0");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&message_effect.payload_json).expect("JSON"),
            serde_json::json!({ "texto": "Já vou verificar." })
        );

        let notification =
            effect_of_transition(&produced(poph_fluxos::ir::Effect::NotifyWhatsApp {
                para: "5511999990000".to_owned(),
                mensagem: "Cliente pediu visita.".to_owned(),
            }));
        assert_eq!(notification.tipo, "notificar_whatsapp");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&notification.payload_json).expect("JSON"),
            serde_json::json!({ "para": "5511999990000", "mensagem": "Cliente pediu visita." })
        );

        let forward = effect_of_transition(&produced(poph_fluxos::ir::Effect::Encaminhar {
            departamento: "Comercial".to_owned(),
        }));
        assert_eq!(forward.tipo, "encaminhar");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&forward.payload_json).expect("JSON"),
            serde_json::json!({ "departamento": "Comercial" })
        );
    }
}
