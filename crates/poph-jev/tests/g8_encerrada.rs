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
    AvaliadorGates, EstadoJev, G8, G8_CONTINUA, G8_DESPEDIDA_PADRAO, G8_ENCERRADA, Gate, IdGate,
    JevFalso, Limiares, MensagemJev, MotorJev, OrigemDecisao, Resposta, despedida_do_agente,
};
use poph_types::GatesJev;

fn estado(mensagens: Vec<MensagemJev>) -> EstadoJev {
    EstadoJev::novo("whatsapp").com_mensagens(mensagens)
}

fn com_agente() -> EstadoJev {
    estado(vec![
        MensagemJev::agente("Mais alguma dúvida?"),
        MensagemJev::cliente("era só isso, obrigado"),
    ])
}

fn resposta(escolha: &str, confianca: f64) -> Resposta {
    Resposta {
        escolha: Some(escolha.to_owned()),
        confianca,
        ..Resposta::default()
    }
}

#[test]
fn pergunta_tem_as_duas_opcoes_e_so_se_aplica_com_fala_do_agente() {
    let gate = G8;
    assert_eq!(gate.id(), IdGate::G8);

    let pergunta = gate.pergunta(&com_agente());
    let opcoes: Vec<&str> = pergunta.criteria.keys().map(String::as_str).collect();
    assert_eq!(
        opcoes,
        vec![G8_CONTINUA, G8_ENCERRADA],
        "as duas opções, em ordem"
    );

    assert!(
        gate.aplicavel(&com_agente()),
        "há fala do agente antes do cliente"
    );
    assert!(
        !gate.aplicavel(&estado(vec![MensagemJev::cliente("obrigado")])),
        "primeira mensagem da conversa não fecha: não há o que encerrar"
    );
    assert!(
        !gate.aplicavel(&estado(vec![MensagemJev::agente("Olá!")])),
        "sem mensagem do cliente para julgar, não se aplica"
    );
}

#[test]
fn so_encerrada_com_confianca_no_limiar_vira_decisao() {
    let gate = G8;
    let limiares = gate.limiares();
    assert_eq!(
        limiares,
        Limiares::novos(0.8, 0.0),
        "limiar mais alto: fechar por engano é o caro"
    );

    let acima = gate.interpretar(&com_agente(), Some(&resposta(G8_ENCERRADA, 0.9)), limiares);
    assert_eq!(acima.origem, OrigemDecisao::Jev);
    assert_eq!(acima.escolha.as_deref(), Some(G8_ENCERRADA));

    let no_limiar = gate.interpretar(&com_agente(), Some(&resposta(G8_ENCERRADA, 0.8)), limiares);
    assert_eq!(
        no_limiar.escolha.as_deref(),
        Some(G8_ENCERRADA),
        "0,8 exato passa"
    );

    let abaixo = gate.interpretar(&com_agente(), Some(&resposta(G8_ENCERRADA, 0.79)), limiares);
    assert_eq!(abaixo.origem, OrigemDecisao::Fallback);
    assert_eq!(
        abaixo.escolha.as_deref(),
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
    let decisao = G8.interpretar(
        &com_agente(),
        Some(&resposta(G8_CONTINUA, 0.95)),
        G8.limiares(),
    );
    assert_eq!(decisao.origem, OrigemDecisao::Jev);
    assert_eq!(decisao.escolha.as_deref(), Some(G8_CONTINUA));
}

#[test]
fn resposta_fora_do_contrato_e_ausente_viram_continua() {
    let limiares = G8.limiares();
    let fora = G8.interpretar(&com_agente(), Some(&resposta("talvez", 1.0)), limiares);
    assert_eq!(fora.origem, OrigemDecisao::Fallback);
    assert_eq!(fora.escolha.as_deref(), Some(G8_CONTINUA));

    let ausente = G8.interpretar(&com_agente(), None, limiares);
    assert_eq!(ausente.origem, OrigemDecisao::Fallback);
    assert_eq!(ausente.escolha.as_deref(), Some(G8_CONTINUA));
    assert_eq!(ausente.motivo.as_deref(), Some("sem resposta do Jev"));
}

#[test]
fn o_motor_so_pergunta_o_g8_com_a_flag_ligada() {
    let jev = Arc::new(JevFalso::com_escolhas(BTreeMap::from([(
        "g8_encerrada".to_owned(),
        G8_ENCERRADA.to_owned(),
    )])));

    let desligado = MotorJev::novo(jev.clone(), GatesJev::default(), "jev-latest");
    let _ = futures_block(desligado.avaliar(com_agente(), None));
    assert_eq!(jev.chamadas(), 0, "com o G8 desligado, nenhuma pergunta");

    let ligado = MotorJev::novo(
        jev.clone(),
        GatesJev {
            g8: true,
            ..GatesJev::default()
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
    assert_eq!(despedida_do_agente("{}"), None, "sem campo: padrão do gate");
    assert_eq!(despedida_do_agente(""), None);
    assert_eq!(
        despedida_do_agente("não é json"),
        None,
        "JSON inválido não derruba o turno"
    );
    assert_eq!(
        despedida_do_agente(r#"{"despedida": 42}"#),
        None,
        "campo com outro tipo é ignorado"
    );
    assert_eq!(
        despedida_do_agente(r#"{"despedida": ""}"#),
        Some(String::new()),
        "vazio = encerrar sem mensagem"
    );
    assert_eq!(
        despedida_do_agente(r#"{"despedida": "  Até mais!  "}"#),
        Some("Até mais!".to_owned())
    );
    let longa = "x".repeat(900);
    let lida = despedida_do_agente(&format!(r#"{{"despedida": "{longa}"}}"#)).unwrap_or_default();
    assert_eq!(lida.chars().count(), poph_jev::flags::MAX_CHARS_DESPEDIDA);
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
