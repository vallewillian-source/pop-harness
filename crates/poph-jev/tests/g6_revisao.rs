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
    AvaliadorGates, EstadoJev, G6, G6_APROVADA, G6_REVISAR, G6_REVISAR_ESCOPO, G6_REVISAR_LONGA,
    G6_REVISAR_PRECO, G6_REVISAR_TOM, G6_REVISAR_TRANSFERENCIA, Gate, IdGate, JevFalso,
    MensagemJev, MotorJev, OrigemDecisao, Resposta, TipoPergunta, eval::ajuste_de_gate,
    g6_orientacao, g6_precisa_revisar,
};
use poph_types::GatesJev;

fn estado(resposta: &str) -> EstadoJev {
    EstadoJev::novo("whatsapp")
        .com_mensagens(vec![
            MensagemJev::agente("Qual é a sua dúvida?"),
            MensagemJev::cliente("quanto custa para dedetizar baratas?"),
        ])
        .com_capacidades(true, true)
        .com_resposta_candidata(resposta)
}

fn resposta(escolha: &str, confianca: f64) -> Resposta {
    let mut item = Resposta {
        escolha: Some(escolha.to_owned()),
        confianca,
        ..Resposta::default()
    };
    item.probabilidades.insert(escolha.to_owned(), confianca);
    item
}

// ───────────────────────────── pergunta e aplicabilidade ─────────────────────────────

/// Seis opções: `aprovada` e os cinco motivos de revisão.
#[test]
fn pergunta_tem_aprovada_e_os_cinco_motivos() {
    let pergunta = G6.pergunta(&estado("Sai R$ 180,00."));
    assert_eq!(pergunta.tipo, TipoPergunta::Choice);
    assert!(!pergunta.instructions.is_empty());
    let opcoes: Vec<&String> = pergunta.criteria.keys().collect();
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
    let sem_resposta = EstadoJev::novo("whatsapp")
        .com_mensagens(vec![MensagemJev::cliente("oi")])
        .com_capacidades(true, true);
    assert!(!G6.aplicavel(&sem_resposta));
    assert!(G6.aplicavel(&estado("Sai R$ 180,00.")));
}

/// A resposta em revisão é truncada, para o pedido ao Jev não crescer com o texto.
#[test]
fn resposta_em_revisao_e_truncada() {
    let longa = "a".repeat(poph_jev::MAX_CHARS_RESPOSTA + 50);
    let estado = EstadoJev::novo("whatsapp").com_resposta_candidata(longa);
    let texto = estado.resposta_candidata.unwrap_or_default();
    assert_eq!(texto.chars().count(), poph_jev::MAX_CHARS_RESPOSTA + 1);
    assert!(texto.ends_with('…'));
}

// ───────────────────────────── decisão ─────────────────────────────

/// `aprovada` com confiança no limiar é decisão confiável e mantém a resposta.
#[test]
fn aprovada_e_decisao_confiavel() {
    let decisao = G6.interpretar(
        &estado("Sai R$ 180,00 por visita."),
        Some(&resposta(G6_APROVADA, 0.93)),
        G6.limiares(),
    );
    assert_eq!(decisao.origem, OrigemDecisao::Jev);
    assert_eq!(decisao.escolha.as_deref(), Some(G6_APROVADA));
    assert_eq!(decisao.motivo, None);
}

/// Cada motivo de revisão é decisão confiável, com a orientação fixa correspondente.
#[test]
fn cada_motivo_de_revisao_e_decisao_confiavel_com_orientacao() {
    for motivo in G6_REVISAR {
        let decisao = G6.interpretar(
            &estado("Amanhã mandamos alguém."),
            Some(&resposta(motivo, 0.88)),
            G6.limiares(),
        );
        assert_eq!(decisao.origem, OrigemDecisao::Jev, "{motivo}");
        assert_eq!(decisao.escolha.as_deref(), Some(motivo));
        assert!(g6_precisa_revisar(motivo));
        let orientacao = g6_orientacao(motivo);
        assert!(
            orientacao.starts_with("Revise a resposta anterior"),
            "{motivo}"
        );
    }
    assert!(!g6_precisa_revisar(G6_APROVADA));
}

/// Confiança abaixo do limiar (0,5) vira fallback que **aprova**: o comportamento de hoje.
#[test]
fn confianca_baixa_aprova() {
    let decisao = G6.interpretar(
        &estado("Sai R$ 180,00."),
        Some(&resposta(G6_REVISAR_PRECO, 0.45)),
        G6.limiares(),
    );
    assert_eq!(decisao.origem, OrigemDecisao::Fallback);
    assert_eq!(decisao.escolha.as_deref(), Some(G6_APROVADA));
    let motivo = decisao.motivo.unwrap_or_default();
    assert!(motivo.contains("abaixo do limiar"), "{motivo}");
}

/// Opção fora do contrato (o transporte pode devolver qualquer coisa) cai no fallback que aprova.
#[test]
fn opcao_fora_do_contrato_aprova() {
    let decisao = G6.interpretar(
        &estado("Sai R$ 180,00."),
        Some(&resposta("reescrever_tudo", 1.0)),
        G6.limiares(),
    );
    assert_eq!(decisao.origem, OrigemDecisao::Fallback);
    assert_eq!(decisao.escolha.as_deref(), Some(G6_APROVADA));
    let motivo = decisao.motivo.unwrap_or_default();
    assert!(motivo.contains("fora das opções"), "{motivo}");
}

/// Sem resposta do Jev, fallback que aprova, com o motivo.
#[test]
fn sem_resposta_do_jev_aprova() {
    let decisao = G6.interpretar(&estado("Sai R$ 180,00."), None, G6.limiares());
    assert_eq!(decisao.origem, OrigemDecisao::Fallback);
    assert_eq!(decisao.escolha.as_deref(), Some(G6_APROVADA));
    assert_eq!(decisao.motivo.as_deref(), Some("sem resposta do Jev"));
}

// ───────────────────────────── fora do lote ─────────────────────────────

/// O lote normal não pergunta o G6 (o estado do turno não tem resposta em revisão); a chamada só do
/// G6 pergunta **uma** coisa e devolve a decisão.
#[tokio::test]
async fn g6_e_perguntado_fora_do_lote_so_com_resposta_em_revisao() {
    let mut escolhas: BTreeMap<String, String> = BTreeMap::new();
    escolhas.insert("g6_revisao".to_owned(), G6_REVISAR_PRECO.to_owned());
    let jev = Arc::new(JevFalso::com_escolhas(escolhas));
    let globais = GatesJev {
        g6: true,
        ..GatesJev::default()
    };
    let transporte: Arc<dyn poph_jev::TransporteJev> = jev.clone();
    let avaliador = MotorJev::novo(transporte, globais, "jev-latest");

    // 1. Lote normal, sem resposta em revisão: o G6 não se aplica e nenhuma pergunta sai.
    let sem_resposta = EstadoJev::novo("whatsapp")
        .com_mensagens(vec![MensagemJev::cliente("quanto custa?")])
        .com_capacidades(true, true);
    let lote = avaliador.avaliar(sem_resposta, None).await;
    assert!(lote.decisoes.is_empty(), "{:?}", lote.decisoes);
    assert!(jev.pedidos().is_empty(), "nenhum pedido ao Jev");

    // 2. Chamada só do G6: uma pergunta, uma decisão.
    let resultado = avaliador
        .avaliar(estado("Sai R$ 180,00."), Some(ajuste_de_gate(IdGate::G6)))
        .await;
    let pedidos = jev.pedidos();
    assert_eq!(pedidos.len(), 1);
    assert_eq!(
        pedidos[0]
            .perguntas
            .keys()
            .cloned()
            .collect::<Vec<String>>(),
        vec!["g6_revisao".to_owned()]
    );
    assert_eq!(
        resultado.escolha_confiavel(IdGate::G6),
        Some(G6_REVISAR_PRECO)
    );
    // A resposta em revisão vai no estado que o Jev lê.
    assert_eq!(
        pedidos[0].estado["resposta_candidata"].as_str(),
        Some("Sai R$ 180,00.")
    );
}

/// O estado sem resposta em revisão não muda de forma (os conjuntos já medidos continuam iguais).
#[test]
fn estado_sem_resposta_nao_leva_o_campo() {
    let estado = EstadoJev::novo("whatsapp").com_mensagens(vec![MensagemJev::cliente("oi")]);
    let json = estado.para_json();
    assert!(json.get("resposta_candidata").is_none());
}

/// As orientações são distintas por motivo (o pedido de reescrita não é genérico).
#[test]
fn orientacoes_sao_distintas_por_motivo() {
    let textos: Vec<&str> = [
        G6_REVISAR_PRECO,
        G6_REVISAR_TRANSFERENCIA,
        G6_REVISAR_ESCOPO,
        G6_REVISAR_LONGA,
        G6_REVISAR_TOM,
    ]
    .into_iter()
    .map(g6_orientacao)
    .collect();
    let mut unicos = textos.clone();
    unicos.sort_unstable();
    unicos.dedup();
    assert_eq!(unicos.len(), textos.len(), "{textos:?}");
}
