//! Testes do lote único de perguntas, do cache e do fallback (HR-28).
//!
//! O Jev é falso e local: nenhuma chamada de rede, nenhuma chave real. O que se prova aqui é o
//! comportamento do motor, não a qualidade das decisões (isso é `poph jev eval` com conjunto
//! rotulado, em cada tarefa de gate).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use poph_jev::{
    AvaliadorGates, DecisionOrigin, G1_CONTINUE, G1_TRANSFER, G2_PROCESS, G3_BASE, G3_NOTHING,
    GateOverrides, IdGate, JevError, JevFalso, JevMessage, JevState, MotorJev, OpcoesJev,
};
use poph_types::JevGates;

/// Estado que faz G1, G2 e G3 se aplicarem (e G4, com dois departamentos).
fn estado() -> JevState {
    JevState::new("cli")
        .with_messages([
            JevMessage::from_customer("oi, boa tarde"),
            JevMessage::from_agent("Boa tarde! Como posso ajudar?"),
            JevMessage::from_customer("queria saber o prazo de entrega"),
        ])
        .with_handoff(vec!["cliente pede segunda via".to_owned()], true)
        .with_capabilities(true, true)
        .with_departments(vec!["Comercial".to_owned(), "Suporte".to_owned()])
}

fn gates(g1: bool, g2: bool, g3: bool, g4: bool) -> JevGates {
    JevGates {
        g1,
        g2,
        g3,
        g4,
        ..JevGates::default()
    }
}

fn escolhas(itens: &[(&str, &str)]) -> BTreeMap<String, String> {
    itens
        .iter()
        .map(|(id, escolha)| ((*id).to_owned(), (*escolha).to_owned()))
        .collect()
}

fn motor(jev: Arc<JevFalso>, globais: JevGates) -> Arc<MotorJev> {
    Arc::new(MotorJev::novo(jev, globais, "jev-latest"))
}

/// Aceite 1: três gates ligados geram **uma** requisição, e a resposta é distribuída certo.
#[tokio::test]
async fn tres_gates_ligados_geram_uma_requisicao_e_distribuem_as_respostas() {
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[
        ("g1_transferir", G1_CONTINUE),
        ("g2_esperar", G2_PROCESS),
        ("g3_precisa", G3_BASE),
    ])));
    let avaliador = motor(Arc::clone(&jev), gates(true, true, true, false));

    let resultado = avaliador.avaliar(estado(), None).await;

    assert_eq!(jev.chamadas(), 1, "o lote é um pedido só");
    let pedido = &jev.pedidos()[0];
    assert_eq!(pedido.perguntas.len(), 3, "três perguntas no mesmo pedido");
    assert_eq!(pedido.modelo, "jev-latest");
    assert_eq!(resultado.chamada.perguntas, 3);
    assert_eq!(resultado.chamada.tokens, 12);
    assert!(!resultado.falhou());

    assert_eq!(resultado.escolha(IdGate::G1), Some(G1_CONTINUE));
    assert_eq!(resultado.escolha(IdGate::G2), Some(G2_PROCESS));
    assert_eq!(resultado.escolha(IdGate::G3), Some(G3_BASE));
    assert!(
        resultado.decisao(IdGate::G4).is_none(),
        "G4 desligado não pergunta"
    );
    assert!(
        resultado
            .decisoes
            .iter()
            .all(|decisao| decisao.origin == DecisionOrigin::Jev)
    );
}

/// O estado vai no pedido: conversa, política e capacidades.
#[tokio::test]
async fn o_pedido_leva_o_estado_da_conversa() {
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[(
        "g1_transferir",
        G1_CONTINUE,
    )])));
    let avaliador = motor(Arc::clone(&jev), gates(true, false, false, false));
    avaliador.avaliar(estado(), None).await;

    let enviado = &jev.pedidos()[0].estado;
    assert_eq!(enviado["canal"], "cli");
    assert_eq!(
        enviado["conversa"]["mensagens"][2]["texto"],
        "queria saber o prazo de entrega"
    );
    assert_eq!(enviado["politica"]["transferencia_habilitada"], true);
    assert_eq!(enviado["capacidades"]["base_de_conhecimento"], true);
}

/// Aceite: flag desligada não gera pergunta — e sem gate ligado o motor nem chama o Jev.
#[tokio::test]
async fn sem_gate_ligado_nenhuma_pergunta_e_enviada() {
    let jev = Arc::new(JevFalso::com_escolhas(BTreeMap::new()));
    let avaliador = motor(Arc::clone(&jev), JevGates::default());

    let resultado = avaliador.avaliar(estado(), None).await;

    assert_eq!(jev.chamadas(), 0);
    assert!(resultado.decisoes.is_empty());
    assert!(!resultado.falhou());
}

/// Aceite: cache evita a mesma pergunta no mesmo estado.
#[tokio::test]
async fn cache_evita_pergunta_repetida_no_mesmo_estado() {
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[(
        "g1_transferir",
        G1_TRANSFER,
    )])));
    let avaliador = motor(Arc::clone(&jev), gates(true, false, false, false));

    let primeiro = avaliador.avaliar(estado(), None).await;
    let segundo = avaliador.avaliar(estado(), None).await;

    assert_eq!(jev.chamadas(), 1, "a segunda vez veio do cache");
    assert_eq!(primeiro.escolha(IdGate::G1), Some(G1_TRANSFER));
    assert_eq!(segundo.escolha(IdGate::G1), Some(G1_TRANSFER));
    assert_eq!(
        segundo.decisao(IdGate::G1).unwrap().origin,
        DecisionOrigin::Cache
    );
    assert_eq!(segundo.chamada.decididas_por_cache, 1);
}

/// Estado diferente não reaproveita a decisão.
#[tokio::test]
async fn estado_diferente_nao_reaproveita_a_decisao() {
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[(
        "g1_transferir",
        G1_CONTINUE,
    )])));
    let avaliador = motor(Arc::clone(&jev), gates(true, false, false, false));

    avaliador.avaliar(estado(), None).await;
    let outro = estado().with_messages([JevMessage::from_customer("quero falar com um atendente")]);
    avaliador.avaliar(outro, None).await;

    assert_eq!(jev.chamadas(), 2);
}

/// Aceite: timeout e erro fazem todos os gates caírem no fallback — o turno não é derrubado.
#[tokio::test]
async fn falha_do_jev_leva_todos_os_gates_ao_fallback() {
    let jev = Arc::new(JevFalso::falhando(JevError::Status {
        code: 503,
        motivo: "Service Unavailable".to_owned(),
    }));
    let avaliador = motor(Arc::clone(&jev), gates(true, true, false, false));

    let resultado = avaliador.avaliar(estado(), None).await;

    assert!(resultado.falhou());
    assert_eq!(
        resultado.chamada.falha.as_deref(),
        Some("o Jev respondeu 503 (Service Unavailable)")
    );
    assert_eq!(resultado.decisoes.len(), 2);
    assert!(
        resultado
            .decisoes
            .iter()
            .all(|decisao| decisao.origin == DecisionOrigin::Fallback)
    );
    // Fallback é o comportamento sem o Jev, não uma escolha inventada.
    assert_eq!(resultado.escolha(IdGate::G1), Some(G1_CONTINUE));
    assert!(resultado.escolha_confiavel(IdGate::G1).is_none());
}

/// Prazo do lote estourado também cai no fallback, sem travar o chamador.
#[tokio::test]
async fn prazo_do_lote_estourado_cai_no_fallback() {
    let jev = Arc::new(
        JevFalso::com_escolhas(escolhas(&[("g1_transferir", G1_TRANSFER)]))
            .com_atraso(Duration::from_secs(5)),
    );
    let avaliador = Arc::new(
        MotorJev::novo(
            Arc::clone(&jev) as Arc<dyn poph_jev::TransporteJev>,
            gates(true, false, false, false),
            "jev-latest",
        )
        .com_opcoes(OpcoesJev {
            tempo_maximo: Duration::from_millis(200),
            ..OpcoesJev::default()
        }),
    );

    let resultado = avaliador.avaliar(estado(), None).await;

    assert!(resultado.falhou());
    assert!(
        resultado
            .chamada
            .falha
            .as_deref()
            .unwrap_or_default()
            .contains("não respondeu")
    );
    assert_eq!(
        resultado.decisao(IdGate::G1).unwrap().origin,
        DecisionOrigin::Fallback
    );
}

/// Resposta sem escolha (score/noul), fora das opções ou com confiança baixa: fallback com motivo.
#[tokio::test]
async fn resposta_ambigua_cai_no_fallback_com_o_motivo() {
    let jev = Arc::new(JevFalso::novo(|pedido| {
        let mut respostas = BTreeMap::new();
        for id in pedido.perguntas.keys() {
            respostas.insert(
                id.clone(),
                poph_jev::JevResponse {
                    choice: Some("opcao_que_nao_existe".to_owned()),
                    confidence: 0.9,
                    ..poph_jev::JevResponse::default()
                },
            );
        }
        Ok(poph_jev::BatchResponse {
            answers: respostas,
            tokens: 3,
            ..poph_jev::BatchResponse::default()
        })
    }));
    let avaliador = motor(Arc::clone(&jev), gates(true, false, false, false));

    let resultado = avaliador.avaliar(estado(), None).await;
    let decisao = resultado.decisao(IdGate::G1).expect("decisão do G1");

    assert_eq!(decisao.origin, DecisionOrigin::Fallback);
    assert!(
        decisao
            .motivo
            .as_deref()
            .unwrap_or_default()
            .contains("fora das opções"),
        "motivo: {:?}",
        decisao.motivo
    );
    assert_eq!(
        decisao.confidence, 0.9,
        "a confiança do Jev fica registrada mesmo no fallback"
    );
}

/// Confiança abaixo do limiar do gate também é abstenção (não vira decisão).
#[tokio::test]
async fn confianca_abaixo_do_limiar_e_abstencao() {
    let jev = Arc::new(JevFalso::novo(|pedido| {
        let mut respostas = BTreeMap::new();
        for id in pedido.perguntas.keys() {
            respostas.insert(
                id.clone(),
                poph_jev::JevResponse {
                    choice: Some(G1_TRANSFER.to_owned()),
                    confidence: 0.55,
                    ..poph_jev::JevResponse::default()
                },
            );
        }
        Ok(poph_jev::BatchResponse {
            answers: respostas,
            tokens: 1,
            ..poph_jev::BatchResponse::default()
        })
    }));
    let avaliador = motor(Arc::clone(&jev), gates(true, false, false, false));

    let resultado = avaliador.avaliar(estado(), None).await;
    let decisao = resultado.decisao(IdGate::G1).unwrap();

    assert_eq!(decisao.origin, DecisionOrigin::Fallback);
    assert!(
        decisao
            .motivo
            .as_deref()
            .unwrap_or_default()
            .contains("confiança 0.55")
    );
    assert_eq!(resultado.escolha(IdGate::G1), Some(G1_CONTINUE));
}

/// Gate que não se aplica ao estado fica de fora do pedido (agente sem transferência, G1 não pergunta).
#[tokio::test]
async fn gate_que_nao_se_aplica_nao_e_perguntado() {
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[(
        "g1_transferir",
        G1_TRANSFER,
    )])));
    let avaliador = motor(Arc::clone(&jev), gates(true, false, false, false));
    let sem_handoff = estado().with_handoff(Vec::new(), false);

    let resultado = avaliador.avaliar(sem_handoff, None).await;

    assert_eq!(jev.chamadas(), 0);
    assert!(resultado.decisoes.is_empty());
}

/// Ajuste do agente sobre a configuração global: só o que ele declara muda.
#[tokio::test]
async fn ajuste_do_agente_sobrepoe_so_o_que_declara() {
    let jev = Arc::new(JevFalso::com_escolhas(BTreeMap::new()));
    let avaliador = motor(Arc::clone(&jev), gates(true, true, false, false));

    let ajuste = GateOverrides::from_json(r#"{"gates": {"g1": false}}"#).expect("ajuste do agente");
    let efetivos = avaliador.gates_efetivos(Some(ajuste));

    assert!(!efetivos.g1, "o agente desligou o G1");
    assert!(efetivos.g2, "o que ele não declarou herda a global");
    assert!(!efetivos.g3);

    // `{"ativo": false}` desliga tudo no agente.
    let desligado = GateOverrides::from_json(r#"{"ativo": false}"#).unwrap();
    assert_eq!(
        avaliador.gates_efetivos(Some(desligado)),
        JevGates::default()
    );
}

/// O mesmo estado com dois gates gera duas perguntas no mesmo pedido (não duas chamadas).
#[tokio::test]
async fn dois_gates_no_mesmo_pedido_e_uma_chamada() {
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[
        ("g1_transferir", G1_CONTINUE),
        ("g3_precisa", G3_NOTHING),
    ])));
    let avaliador = motor(Arc::clone(&jev), gates(true, false, true, false));

    let resultado = avaliador.avaliar(estado(), None).await;

    assert_eq!(jev.chamadas(), 1);
    assert_eq!(jev.pedidos()[0].perguntas.len(), 2);
    assert_eq!(resultado.escolha(IdGate::G3), Some(G3_NOTHING));
}
