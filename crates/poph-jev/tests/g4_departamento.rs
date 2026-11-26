//! G4 — departamento que recebe a transferência (HR-32): o que o gate decide e o que o código
//! recusa.
//!
//! O gate decide **um nome** entre os departamentos candidatos; a validação contra o banco (org,
//! situação ativa) e o fallback para o padrão do agente são do turno (`poph-agent`, testes
//! `g4_departamento.rs`). Aqui se prova o contrato do gate: nome válido vira decisão confiável,
//! nome fora das opções e confiança baixa viram fallback **com motivo**, e sem candidatos o gate nem
//! pergunta.

use std::collections::BTreeMap;

use poph_jev::{
    AvaliadorGates, BatchResponse, DecisionOrigin, G4, Gate, IdGate, JevFalso, JevMessage,
    JevResponse, JevState, MotorJev, Pedido, eval::ajuste_de_gate, match_candidate,
};

fn estado(departamentos: &[&str], texto: &str) -> JevState {
    JevState::new("whatsapp")
        .with_messages(vec![JevMessage::from_customer(texto)])
        .with_departments(
            departamentos
                .iter()
                .map(|nome| (*nome).to_owned())
                .collect(),
        )
}

fn resposta(escolha: &str, confianca: f64) -> JevResponse {
    let mut item = JevResponse {
        choice: Some(escolha.to_owned()),
        confidence: confianca,
        ..JevResponse::default()
    };
    item.probabilities.insert(escolha.to_owned(), confianca);
    item
}

fn candidatos() -> Vec<String> {
    ["Comercial", "Suporte", "Financeiro"]
        .iter()
        .map(|nome| (*nome).to_owned())
        .collect()
}

// ───────────────────────────── pergunta ─────────────────────────────

/// Uma opção por departamento candidato, no id estável do gate.
#[test]
fn pergunta_tem_uma_opcao_por_departamento() {
    let pergunta = G4.question(&estado(
        &["Comercial", "Suporte", "Financeiro"],
        "chegou quebrado",
    ));
    assert_eq!(pergunta.tipo, poph_jev::QuestionType::Choice);
    assert!(!pergunta.instructions.is_empty());
    let opcoes: Vec<&String> = pergunta.rubric.keys().collect();
    assert_eq!(opcoes, vec!["Comercial", "Financeiro", "Suporte"]);
}

/// Com menos de dois candidatos não há escolha a fazer: o gate não se aplica.
#[test]
fn sem_escolha_possivel_o_gate_nao_se_aplica() {
    assert!(!G4.applies(&estado(&["Suporte"], "chegou quebrado")));
    assert!(!G4.applies(&estado(&[], "chegou quebrado")));
    assert!(G4.applies(&estado(&["Suporte", "Comercial"], "chegou quebrado")));
}

// ───────────────────────────── decisão ─────────────────────────────

/// Escolha válida e confiável vira decisão do Jev, com o nome canônico.
#[test]
fn escolha_valida_vira_decisao_confiavel() {
    let estado = estado(
        &["Comercial", "Suporte", "Financeiro"],
        "o produto chegou quebrado",
    );
    let decisao = G4.interpret(&estado, Some(&resposta("Suporte", 0.94)), G4.thresholds());
    assert_eq!(decisao.choice.as_deref(), Some("Suporte"));
    assert_eq!(decisao.origin, DecisionOrigin::Jev);
    assert_eq!(decisao.motivo, None);
    assert!(decisao.is_reliable());
}

/// O modelo respondendo em caixa diferente casa com o nome cadastrado (e o registro mostra o
/// canônico).
#[test]
fn escolha_com_caixa_diferente_vira_o_nome_canonico() {
    let estado = estado(
        &["Comercial", "Suporte", "Financeiro"],
        "quero a segunda via do boleto",
    );
    let decisao = G4.interpret(&estado, Some(&resposta("financeiro", 0.9)), G4.thresholds());
    assert_eq!(decisao.choice.as_deref(), Some("Financeiro"));
    assert!(decisao.is_reliable());
}

/// Nome que não é candidato **nunca** vira decisão, por mais confiança que o modelo declare.
#[test]
fn escolha_fora_das_opcoes_cai_no_fallback_com_motivo() {
    let estado = estado(
        &["Comercial", "Suporte", "Financeiro"],
        "quero falar do contrato",
    );
    let decisao = G4.interpret(&estado, Some(&resposta("Jurídico", 0.99)), G4.thresholds());
    assert_eq!(decisao.choice, None);
    assert_eq!(decisao.origin, DecisionOrigin::Fallback);
    let motivo = decisao.motivo.unwrap_or_default();
    assert!(
        motivo.contains("não é um departamento ativo"),
        "motivo devia dizer que não é departamento da empresa: {motivo}"
    );
}

/// Departamento de outra empresa pode ter o mesmo nome; se o nome existe aqui, vale o daqui — o que
/// não pode existir é id de fora, e isso o turno barra na leitura do banco.
#[test]
fn confianca_abaixo_do_limiar_cai_no_fallback() {
    let estado = estado(
        &["Comercial", "Suporte", "Financeiro"],
        "quero negociar um desconto",
    );
    let decisao = G4.interpret(&estado, Some(&resposta("Comercial", 0.61)), G4.thresholds());
    assert_eq!(decisao.choice, None);
    assert_eq!(decisao.origin, DecisionOrigin::Fallback);
    let motivo = decisao.motivo.unwrap_or_default();
    assert!(motivo.contains("confiança 0.61"), "motivo: {motivo}");
}

/// Sem resposta (chamada falhou, resposta sem escolha) também é fallback, sem pânico.
#[test]
fn sem_resposta_cai_no_fallback() {
    let estado = estado(&["Comercial", "Suporte"], "oi");
    let decisao = G4.interpret(&estado, None, G4.thresholds());
    assert_eq!(decisao.choice, None);
    assert_eq!(decisao.origin, DecisionOrigin::Fallback);
    assert!(decisao.motivo.unwrap_or_default().contains("sem resposta"));

    let sem_escolha = JevResponse {
        choice: None,
        confidence: 0.9,
        ..JevResponse::default()
    };
    let decisao = G4.interpret(&estado, Some(&sem_escolha), G4.thresholds());
    assert_eq!(decisao.choice, None);
    assert!(!decisao.is_reliable());
}

// ───────────────────────────── casamento de nomes ─────────────────────────────

#[test]
fn casamento_de_nomes_e_estrito_sem_ser_burro() {
    let candidatos = candidatos();
    assert_eq!(
        match_candidate("Suporte", &candidatos).as_deref(),
        Ok("Suporte")
    );
    assert_eq!(
        match_candidate("  Suporte  ", &candidatos).as_deref(),
        Ok("Suporte")
    );
    assert_eq!(
        match_candidate("SUPORTE", &candidatos).as_deref(),
        Ok("Suporte")
    );
    assert!(match_candidate("Jurídico", &candidatos).is_err());
    assert!(match_candidate("", &candidatos).is_err());
    // Ambíguo: dois cadastros que só diferem por caixa (nenhum com a grafia que o modelo usou).
    let ambiguos = vec!["Financeiro".to_owned(), "FINANCEIRO".to_owned()];
    let erro = match_candidate("financeiro", &ambiguos).unwrap_err();
    assert!(erro.contains("ambíguo"), "{erro}");
}

// ───────────────────────────── lote e fallback do motor ─────────────────────────────

/// Com só o G4 ligado, o lote leva só a pergunta do G4; nome inválido no lote vira fallback com a
/// escolha crua preservada (é o que o `poph jev eval` mostra).
#[tokio::test]
async fn lote_do_g4_e_o_fallback_preserva_a_escolha_crua() {
    let jev = std::sync::Arc::new(JevFalso::novo(|pedido: &Pedido| {
        assert_eq!(
            pedido.perguntas.keys().collect::<Vec<_>>(),
            vec![IdGate::G4.question_id()]
        );
        let mut respostas = BTreeMap::new();
        respostas.insert(
            IdGate::G4.question_id().to_owned(),
            resposta("Marketing", 0.95),
        );
        Ok(BatchResponse {
            answers: respostas,
            tokens: 7,
            ..BatchResponse::default()
        })
    }));
    let motor = MotorJev::novo(jev, poph_types::JevGates::default(), "jev-latest");
    let resultado = motor
        .avaliar(
            estado(
                &["Comercial", "Suporte", "Financeiro"],
                "quero o preço do produto",
            ),
            Some(ajuste_de_gate(IdGate::G4)),
        )
        .await;

    let decisao = resultado.decisao(IdGate::G4).expect("o G4 rodou");
    assert_eq!(decisao.choice, None);
    assert_eq!(decisao.origin, DecisionOrigin::Fallback);
    // O que o modelo respondeu fica registrado, mesmo recusado.
    assert_eq!(
        resultado
            .resposta(IdGate::G4)
            .and_then(|bruta| bruta.choice.as_deref()),
        Some("Marketing")
    );
}

/// Gate desligado não gera decisão nenhuma (o turno cai no padrão do agente).
#[tokio::test]
async fn gate_desligado_nao_decide_nada() {
    let jev = std::sync::Arc::new(JevFalso::com_escolhas(BTreeMap::from([(
        IdGate::G4.question_id().to_owned(),
        "Suporte".to_owned(),
    )])));
    let motor = MotorJev::novo(jev, poph_types::JevGates::default(), "jev-latest");
    let resultado = motor
        .avaliar(
            estado(&["Comercial", "Suporte"], "chegou quebrado"),
            Some(poph_jev::GateOverrides::default()),
        )
        .await;
    assert!(resultado.decisao(IdGate::G4).is_none());
}
