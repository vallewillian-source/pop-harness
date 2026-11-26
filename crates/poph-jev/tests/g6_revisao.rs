//! G6 — revisão da resposta antes do envio (HR-34): o que o gate decide.
//!
//! A pergunta é **fora do lote** (só existe depois de o modelo ter produzido o texto) e as opções
//! são `aprovada` mais cinco `revisar_*`, cada uma com o motivo. Aqui se prova o contrato do gate:
//! `aprovada` mantém a resposta; cada `revisar_*` vira decisão confiável com o motivo; confiança
//! baixa, opção fora do contrato e falta de resposta caem no fallback **`aprovada`** (o
//! comportamento de hoje); sem resposta em revisão o gate nem pergunta. Quem regenera e aplica a
//! resposta segura é o `poph-agent` (teste `g6_revisao.rs` de lá).

use std::collections::BTreeMap;
use std::sync::Arc;

use poph_jev::{
    DecisionOrigin, FakeJev, G6, G6_APPROVED, G6_REVIEW, G6_REVIEW_LONG, G6_REVIEW_PRICE,
    G6_REVIEW_SCOPE, G6_REVIEW_TONE, G6_REVIEW_TRANSFER, Gate, GateEvaluator, IdGate, JevEngine,
    JevMessage, JevResponse, JevState, QuestionType, eval::gate_override, g6_guidance,
    g6_needs_review,
};
use poph_types::JevGates;

fn estado(resposta: &str) -> JevState {
    JevState::new("whatsapp")
        .with_messages(vec![
            JevMessage::from_agent("Qual é a sua dúvida?"),
            JevMessage::from_customer("quanto custa para dedetizar baratas?"),
        ])
        .with_capabilities(true, true)
        .with_candidate_response(resposta)
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

// ───────────────────────────── pergunta e aplicabilidade ─────────────────────────────

/// Seis opções: `aprovada` e os cinco motivos de revisão.
#[test]
fn pergunta_tem_aprovada_e_os_cinco_motivos() {
    let pergunta = G6.question(&estado("Sai R$ 180,00."));
    assert_eq!(pergunta.tipo, QuestionType::Choice);
    assert!(!pergunta.instructions.is_empty());
    let opcoes: Vec<&String> = pergunta.rubric.keys().collect();
    assert_eq!(
        opcoes,
        vec![
            "aprovada",
            "revisar_escopo",
            "revisar_longa",
            "revisar_preco",
            "revisar_tom",
            "revisar_transferencia",
        ]
    );
}

/// Sem resposta em revisão não há o que julgar: o gate não se aplica (e não pergunta).
#[test]
fn sem_resposta_em_revisao_o_gate_nao_se_aplica() {
    let sem_resposta = JevState::new("whatsapp")
        .with_messages(vec![JevMessage::from_customer("oi")])
        .with_capabilities(true, true);
    assert!(!G6.applies(&sem_resposta));
    assert!(G6.applies(&estado("Sai R$ 180,00.")));
}

/// A resposta em revisão é truncada, para o pedido ao Jev não crescer com o texto.
#[test]
fn resposta_em_revisao_e_truncada() {
    let longa = "a".repeat(poph_jev::MAX_CANDIDATE_RESPONSE_CHARS + 50);
    let estado = JevState::new("whatsapp").with_candidate_response(longa);
    let texto = estado.candidate_response.unwrap_or_default();
    assert_eq!(
        texto.chars().count(),
        poph_jev::MAX_CANDIDATE_RESPONSE_CHARS + 1
    );
    assert!(texto.ends_with('…'));
}

// ───────────────────────────── decisão ─────────────────────────────

/// `aprovada` com confiança no limiar é decisão confiável e mantém a resposta.
#[test]
fn aprovada_e_decisao_confiavel() {
    let decisao = G6.interpret(
        &estado("Sai R$ 180,00 por visita."),
        Some(&resposta(G6_APPROVED, 0.93)),
        G6.thresholds(),
    );
    assert_eq!(decisao.origin, DecisionOrigin::Jev);
    assert_eq!(decisao.choice.as_deref(), Some(G6_APPROVED));
    assert_eq!(decisao.motivo, None);
}

/// Cada motivo de revisão é decisão confiável, com a orientação fixa correspondente.
#[test]
fn cada_motivo_de_revisao_e_decisao_confiavel_com_orientacao() {
    for motivo in G6_REVIEW {
        let decisao = G6.interpret(
            &estado("Amanhã mandamos alguém."),
            Some(&resposta(motivo, 0.88)),
            G6.thresholds(),
        );
        assert_eq!(decisao.origin, DecisionOrigin::Jev, "{motivo}");
        assert_eq!(decisao.choice.as_deref(), Some(motivo));
        assert!(g6_needs_review(motivo));
        let orientacao = g6_guidance(motivo);
        assert!(
            orientacao.starts_with("Revise a resposta anterior"),
            "{motivo}"
        );
    }
    assert!(!g6_needs_review(G6_APPROVED));
}

/// Confiança abaixo do limiar (0,5) vira fallback que **aprova**: o comportamento de hoje.
#[test]
fn confianca_baixa_aprova() {
    let decisao = G6.interpret(
        &estado("Sai R$ 180,00."),
        Some(&resposta(G6_REVIEW_PRICE, 0.45)),
        G6.thresholds(),
    );
    assert_eq!(decisao.origin, DecisionOrigin::Fallback);
    assert_eq!(decisao.choice.as_deref(), Some(G6_APPROVED));
    let motivo = decisao.motivo.unwrap_or_default();
    assert!(motivo.contains("abaixo do limiar"), "{motivo}");
}

/// Opção fora do contrato (o transporte pode devolver qualquer coisa) cai no fallback que aprova.
#[test]
fn opcao_fora_do_contrato_aprova() {
    let decisao = G6.interpret(
        &estado("Sai R$ 180,00."),
        Some(&resposta("reescrever_tudo", 1.0)),
        G6.thresholds(),
    );
    assert_eq!(decisao.origin, DecisionOrigin::Fallback);
    assert_eq!(decisao.choice.as_deref(), Some(G6_APPROVED));
    let motivo = decisao.motivo.unwrap_or_default();
    assert!(motivo.contains("fora das opções"), "{motivo}");
}

/// Sem resposta do Jev, fallback que aprova, com o motivo.
#[test]
fn sem_resposta_do_jev_aprova() {
    let decisao = G6.interpret(&estado("Sai R$ 180,00."), None, G6.thresholds());
    assert_eq!(decisao.origin, DecisionOrigin::Fallback);
    assert_eq!(decisao.choice.as_deref(), Some(G6_APPROVED));
    assert_eq!(decisao.motivo.as_deref(), Some("sem resposta do Jev"));
}

// ───────────────────────────── fora do lote ─────────────────────────────

/// O lote normal não pergunta o G6 (o estado do turno não tem resposta em revisão); a chamada só do
/// G6 pergunta **uma** coisa e devolve a decisão.
#[tokio::test]
async fn g6_e_perguntado_fora_do_lote_so_com_resposta_em_revisao() {
    let mut escolhas: BTreeMap<String, String> = BTreeMap::new();
    escolhas.insert("g6_revisao".to_owned(), G6_REVIEW_PRICE.to_owned());
    let jev = Arc::new(FakeJev::with_choices(escolhas));
    let globais = JevGates {
        g6: true,
        ..JevGates::default()
    };
    let transporte: Arc<dyn poph_jev::JevTransport> = jev.clone();
    let avaliador = JevEngine::new(transporte, globais, "jev-latest");

    // 1. Lote normal, sem resposta em revisão: o G6 não se aplica e nenhuma pergunta sai.
    let sem_resposta = JevState::new("whatsapp")
        .with_messages(vec![JevMessage::from_customer("quanto custa?")])
        .with_capabilities(true, true);
    let lote = avaliador.evaluate(sem_resposta, None).await;
    assert!(lote.decisions.is_empty(), "{:?}", lote.decisions);
    assert!(jev.requests().is_empty(), "nenhum pedido ao Jev");

    // 2. Chamada só do G6: uma pergunta, uma decisão.
    let resultado = avaliador
        .evaluate(estado("Sai R$ 180,00."), Some(gate_override(IdGate::G6)))
        .await;
    let pedidos = jev.requests();
    assert_eq!(pedidos.len(), 1);
    assert_eq!(
        pedidos[0]
            .questions
            .keys()
            .cloned()
            .collect::<Vec<String>>(),
        vec!["g6_revisao".to_owned()]
    );
    assert_eq!(resultado.reliable_choice(IdGate::G6), Some(G6_REVIEW_PRICE));
    // A resposta em revisão vai no estado que o Jev lê.
    assert_eq!(
        pedidos[0].estado["resposta_candidata"].as_str(),
        Some("Sai R$ 180,00.")
    );
}

/// O estado sem resposta em revisão não muda de forma (os conjuntos já medidos continuam iguais).
#[test]
fn estado_sem_resposta_nao_leva_o_campo() {
    let estado = JevState::new("whatsapp").with_messages(vec![JevMessage::from_customer("oi")]);
    let json = estado.to_json();
    assert!(json.get("resposta_candidata").is_none());
}

/// As orientações são distintas por motivo (o pedido de reescrita não é genérico).
#[test]
fn orientacoes_sao_distintas_por_motivo() {
    let textos: Vec<&str> = [
        G6_REVIEW_PRICE,
        G6_REVIEW_TRANSFER,
        G6_REVIEW_SCOPE,
        G6_REVIEW_LONG,
        G6_REVIEW_TONE,
    ]
    .into_iter()
    .map(g6_guidance)
    .collect();
    let mut unicos = textos.clone();
    unicos.sort_unstable();
    unicos.dedup();
    assert_eq!(unicos.len(), textos.len(), "{textos:?}");
}
