//! Contrato do G8 (HR-36): conversa encerrada pelo cliente.
//!
//! O que se prova aqui, sem rede:
//! 1. a pergunta tem as duas opções (`encerrada`, `continua`) e só existe com fala do agente antes
//!    da última do cliente: a primeira mensagem de uma conversa nunca fecha;
//! 2. só `encerrada` com confiança no limiar (0,8) vira decisão; qualquer outra coisa — confiança
//!    baixa, opção fora do contrato, resposta ausente — cai em `continua`, que é o comportamento de
//!    hoje;
//! 3. o motor manda a pergunta no lote só com o G8 ligado;
//! 4. a despedida configurada no agente é lida com as regras do JSON de operador (ausente, vazia,
//!    inválida, longa).

use std::collections::BTreeMap;
use std::sync::Arc;

use poph_jev::{
    AvaliadorGates, DecisionOrigin, G8, G8_CONTINUA, G8_DESPEDIDA_PADRAO, G8_ENCERRADA, Gate,
    IdGate, JevFalso, JevMessage, JevResponse, JevState, MotorJev, Thresholds, farewell_from_agent,
};
use poph_types::JevGates;

fn estado(mensagens: Vec<JevMessage>) -> JevState {
    JevState::new("whatsapp").with_messages(mensagens)
}

fn com_agente() -> JevState {
    estado(vec![
        JevMessage::from_agent("Mais alguma dúvida?"),
        JevMessage::from_customer("era só isso, obrigado"),
    ])
}

fn resposta(escolha: &str, confianca: f64) -> JevResponse {
    JevResponse {
        choice: Some(escolha.to_owned()),
        confidence: confianca,
        ..JevResponse::default()
    }
}

#[test]
fn pergunta_tem_as_duas_opcoes_e_so_se_aplica_com_fala_do_agente() {
    let gate = G8;
    assert_eq!(gate.id(), IdGate::G8);

    let pergunta = gate.question(&com_agente());
    let opcoes: Vec<&str> = pergunta.rubric.keys().map(String::as_str).collect();
    assert_eq!(
        opcoes,
        vec![G8_CONTINUA, G8_ENCERRADA],
        "as duas opções, em ordem"
    );

    assert!(
        gate.applies(&com_agente()),
        "há fala do agente antes do cliente"
    );
    assert!(
        !gate.applies(&estado(vec![JevMessage::from_customer("obrigado")])),
        "primeira mensagem da conversa não fecha: não há o que encerrar"
    );
    assert!(
        !gate.applies(&estado(vec![JevMessage::from_agent("Olá!")])),
        "sem mensagem do cliente para julgar, não se aplica"
    );
}

#[test]
fn so_encerrada_com_confianca_no_limiar_vira_decisao() {
    let gate = G8;
    let limiares = gate.thresholds();
    assert_eq!(
        limiares,
        Thresholds::new(0.8, 0.0),
        "limiar mais alto: fechar por engano é o caro"
    );

    let acima = gate.interpret(&com_agente(), Some(&resposta(G8_ENCERRADA, 0.9)), limiares);
    assert_eq!(acima.origin, DecisionOrigin::Jev);
    assert_eq!(acima.choice.as_deref(), Some(G8_ENCERRADA));

    let no_limiar = gate.interpret(&com_agente(), Some(&resposta(G8_ENCERRADA, 0.8)), limiares);
    assert_eq!(
        no_limiar.choice.as_deref(),
        Some(G8_ENCERRADA),
        "0,8 exato passa"
    );

    let abaixo = gate.interpret(&com_agente(), Some(&resposta(G8_ENCERRADA, 0.79)), limiares);
    assert_eq!(abaixo.origin, DecisionOrigin::Fallback);
    assert_eq!(
        abaixo.choice.as_deref(),
        Some(G8_CONTINUA),
        "abaixo do limiar: conversa segue"
    );
    assert!(
        abaixo
            .motivo
            .unwrap_or_default()
            .contains("abaixo do limiar")
    );
}

#[test]
fn continua_confiante_e_decisao_sem_efeito_de_encerramento() {
    let decisao = G8.interpret(
        &com_agente(),
        Some(&resposta(G8_CONTINUA, 0.95)),
        G8.thresholds(),
    );
    assert_eq!(decisao.origin, DecisionOrigin::Jev);
    assert_eq!(decisao.choice.as_deref(), Some(G8_CONTINUA));
}

#[test]
fn resposta_fora_do_contrato_e_ausente_viram_continua() {
    let limiares = G8.thresholds();
    let fora = G8.interpret(&com_agente(), Some(&resposta("talvez", 1.0)), limiares);
    assert_eq!(fora.origin, DecisionOrigin::Fallback);
    assert_eq!(fora.choice.as_deref(), Some(G8_CONTINUA));

    let ausente = G8.interpret(&com_agente(), None, limiares);
    assert_eq!(ausente.origin, DecisionOrigin::Fallback);
    assert_eq!(ausente.choice.as_deref(), Some(G8_CONTINUA));
    assert_eq!(ausente.motivo.as_deref(), Some("sem resposta do Jev"));
}

#[test]
fn o_motor_so_pergunta_o_g8_com_a_flag_ligada() {
    let jev = Arc::new(JevFalso::com_escolhas(BTreeMap::from([(
        "g8_encerrada".to_owned(),
        G8_ENCERRADA.to_owned(),
    )])));

    let desligado = MotorJev::novo(jev.clone(), JevGates::default(), "jev-latest");
    let _ = futures_block(desligado.avaliar(com_agente(), None));
    assert_eq!(jev.chamadas(), 0, "com o G8 desligado, nenhuma pergunta");

    let ligado = MotorJev::novo(
        jev.clone(),
        JevGates {
            g8: true,
            ..JevGates::default()
        },
        "jev-latest",
    );
    let resultado = futures_block(ligado.avaliar(com_agente(), None));
    assert_eq!(jev.chamadas(), 1);
    let pedido = jev.pedidos().pop().expect("um pedido");
    assert!(pedido.perguntas.contains_key("g8_encerrada"));
    assert_eq!(resultado.escolha(IdGate::G8), Some(G8_ENCERRADA));
}

#[test]
fn despedida_do_agente_respeita_ausente_vazia_invalida_e_longa() {
    assert_eq!(farewell_from_agent("{}"), None, "sem campo: padrão do gate");
    assert_eq!(farewell_from_agent(""), None);
    assert_eq!(
        farewell_from_agent("não é json"),
        None,
        "JSON inválido não derruba o turno"
    );
    assert_eq!(
        farewell_from_agent(r#"{"despedida": 42}"#),
        None,
        "campo com outro tipo é ignorado"
    );
    assert_eq!(
        farewell_from_agent(r#"{"despedida": ""}"#),
        Some(String::new()),
        "vazio = encerrar sem mensagem"
    );
    assert_eq!(
        farewell_from_agent(r#"{"despedida": "  Até mais!  "}"#),
        Some("Até mais!".to_owned())
    );
    let longa = "x".repeat(900);
    let lida = farewell_from_agent(&format!(r#"{{"despedida": "{longa}"}}"#)).unwrap_or_default();
    assert_eq!(lida.chars().count(), poph_jev::flags::MAX_FAREWELL_CHARS);
    assert!(!G8_DESPEDIDA_PADRAO.is_empty());
}

/// Executa um futuro sem runtime do motor: o Jev falso responde na hora.
fn futures_block<T>(futuro: futures::future::BoxFuture<'_, T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime de teste")
        .block_on(futuro)
}
