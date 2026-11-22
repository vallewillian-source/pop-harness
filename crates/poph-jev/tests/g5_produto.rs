//! G5 — qual produto o cliente quer (HR-33): o que o gate decide e o que o código recusa.
//!
//! A pergunta é **fora do lote** (os candidatos são o resultado da busca de produtos do turno) e
//! as opções são os **ids** dos candidatos, mais `nenhum`. Aqui se prova o contrato do gate: id de
//! candidato vira decisão confiável; id inventado, `nenhum`, confiança baixa e falta de resposta
//! viram fallback **com motivo**; sem candidatos o gate nem pergunta. Quem injeta o produto no
//! turno e valida o id de novo é o `poph-agent` (teste `g5_produto.rs` de lá).

use std::collections::BTreeMap;
use std::sync::Arc;

use poph_jev::{
    AvaliadorGates, CandidatoProduto, EstadoJev, G5, G5_NENHUM, Gate, IdGate, JevFalso,
    MensagemJev, MotorJev, OrigemDecisao, Resposta, eval::ajuste_de_gate,
};
use poph_types::GatesJev;

fn candidato(id: &str, nome: &str, detalhe: &str) -> CandidatoProduto {
    CandidatoProduto {
        id: id.to_owned(),
        nome: nome.to_owned(),
        detalhe: detalhe.to_owned(),
    }
}

fn candidatos() -> Vec<CandidatoProduto> {
    vec![
        candidato(
            "p-baratas",
            "Dedetização de baratas",
            "categoria: Pragas; preço: R$ 180,00",
        ),
        candidato(
            "p-cupins",
            "Descupinização de cupins",
            "categoria: Pragas; preço: R$ 350,00",
        ),
        candidato(
            "p-ratos",
            "Controle de ratos",
            "categoria: Pragas; preço: R$ 220,00",
        ),
    ]
}

fn estado(texto: &str, candidatos: Vec<CandidatoProduto>) -> EstadoJev {
    EstadoJev::novo("whatsapp")
        .com_mensagens(vec![MensagemJev::cliente(texto)])
        .com_capacidades(true, true)
        .com_produtos_candidatos(candidatos)
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

// ───────────────────────────── pergunta ─────────────────────────────

/// Uma opção por candidato (pelo id) mais `nenhum`.
#[test]
fn pergunta_tem_uma_opcao_por_candidato_e_nenhum() {
    let pergunta = G5.pergunta(&estado("quanto custa para matar barata?", candidatos()));
    assert_eq!(pergunta.tipo, poph_jev::TipoPergunta::Choice);
    assert!(!pergunta.instructions.is_empty());
    let opcoes: Vec<&String> = pergunta.criteria.keys().collect();
    assert_eq!(opcoes, vec!["nenhum", "p-baratas", "p-cupins", "p-ratos"]);
    // O critério do candidato diz o nome e o detalhe, para o Jev reconhecer o produto.
    let criterio = &pergunta.criteria["p-baratas"];
    assert!(criterio.contains("Dedetização de baratas"), "{criterio}");
    assert!(criterio.contains("R$ 180,00"), "{criterio}");
}

/// Sem candidatos o gate não se aplica (a busca devolveu vazio ou o turno não consultou o catálogo).
#[test]
fn sem_candidato_o_gate_nao_se_aplica() {
    assert!(!G5.aplicavel(&estado("quanto custa?", Vec::new())));
    assert!(G5.aplicavel(&estado("quanto custa?", candidatos())));
}

// ───────────────────────────── decisão ─────────────────────────────

/// Id de um candidato, com confiança acima do limiar, vira decisão confiável.
#[test]
fn id_de_candidato_vira_decisao_confiavel() {
    let estado = estado("quanto custa para matar barata?", candidatos());
    let decisao = G5.interpretar(&estado, Some(&resposta("p-baratas", 0.92)), G5.limiares());
    assert_eq!(decisao.escolha.as_deref(), Some("p-baratas"));
    assert_eq!(decisao.origem, OrigemDecisao::Jev);
    assert_eq!(decisao.motivo, None);
}

/// Id inventado nunca é aceito, nem com confiança 1,0: a lista de candidatos é o limite.
#[test]
fn id_fora_dos_candidatos_e_recusado() {
    let estado = estado("quanto custa para matar barata?", candidatos());
    let decisao = G5.interpretar(
        &estado,
        Some(&resposta("p-produto-que-nao-existe", 1.0)),
        G5.limiares(),
    );
    assert_eq!(decisao.origem, OrigemDecisao::Fallback);
    assert_eq!(decisao.escolha, None);
    let motivo = decisao.motivo.unwrap_or_default();
    assert!(motivo.contains("fora das opções"), "{motivo}");
}

/// `nenhum` é abstenção explícita: os candidatos vão ao LLM, como sem o gate.
#[test]
fn nenhum_e_abstencao_com_motivo() {
    let estado = estado("qual o prazo de entrega?", candidatos());
    let decisao = G5.interpretar(&estado, Some(&resposta(G5_NENHUM, 0.95)), G5.limiares());
    assert_eq!(decisao.origem, OrigemDecisao::Fallback);
    assert_eq!(decisao.escolha, None);
    let motivo = decisao.motivo.unwrap_or_default();
    assert!(motivo.contains("não reconheceu o produto"), "{motivo}");
}

/// Confiança abaixo do limiar (0,7) vira fallback com o motivo.
#[test]
fn confianca_baixa_vira_fallback() {
    let estado = estado("quanto custa para matar barata?", candidatos());
    let decisao = G5.interpretar(&estado, Some(&resposta("p-baratas", 0.55)), G5.limiares());
    assert_eq!(decisao.origem, OrigemDecisao::Fallback);
    assert_eq!(decisao.escolha, None);
    let motivo = decisao.motivo.unwrap_or_default();
    assert!(motivo.contains("abaixo do limiar"), "{motivo}");
}

/// Sem resposta do Jev, fallback com o motivo.
#[test]
fn sem_resposta_vira_fallback() {
    let estado = estado("quanto custa?", candidatos());
    let decisao = G5.interpretar(&estado, None, G5.limiares());
    assert_eq!(decisao.origem, OrigemDecisao::Fallback);
    assert_eq!(decisao.motivo.as_deref(), Some("sem resposta do Jev"));
}

// ───────────────────────────── fora do lote ─────────────────────────────

/// O lote normal não pergunta o G5 (o estado não tem candidatos); a chamada só do G5 pergunta **uma**
/// coisa e devolve a decisão.
#[tokio::test]
async fn g5_e_perguntado_fora_do_lote_so_com_candidatos() {
    let mut escolhas: BTreeMap<String, String> = BTreeMap::new();
    escolhas.insert("g5_produto".to_owned(), "p-baratas".to_owned());
    let jev = Arc::new(JevFalso::com_escolhas(escolhas));
    let globais = GatesJev {
        g5: true,
        ..GatesJev::default()
    };
    let transporte: Arc<dyn poph_jev::TransporteJev> = jev.clone();
    let avaliador = MotorJev::novo(transporte, globais, "jev-latest");

    // 1. Lote normal (sem candidatos): o G5 não se aplica, então nenhuma pergunta é enviada.
    let sem_candidatos = EstadoJev::novo("whatsapp")
        .com_mensagens(vec![MensagemJev::cliente("oi")])
        .com_capacidades(true, true);
    let lote = avaliador.avaliar(sem_candidatos, None).await;
    assert!(lote.decisoes.is_empty(), "{:?}", lote.decisoes);
    assert!(jev.pedidos().is_empty(), "nenhum pedido ao Jev");

    // 2. Chamada só do G5, com candidatos: uma pergunta, uma decisão.
    let com_candidatos = estado("quero dedetizar contra baratas", candidatos());
    let resultado = avaliador
        .avaliar(com_candidatos, Some(ajuste_de_gate(IdGate::G5)))
        .await;
    let pedidos = jev.pedidos();
    assert_eq!(pedidos.len(), 1);
    assert_eq!(
        pedidos[0]
            .perguntas
            .keys()
            .cloned()
            .collect::<Vec<String>>(),
        vec!["g5_produto".to_owned()]
    );
    assert_eq!(resultado.escolha_confiavel(IdGate::G5), Some("p-baratas"));
}

/// O Jev falso pode devolver o id mesmo quando ele não é candidato: o gate recusa e o motor marca
/// fallback (defesa em profundidade com o que o modelo devolveu).
#[test]
fn resposta_com_id_inventado_pelo_transporte_vira_fallback() {
    let estado = estado("quero dedetizar", candidatos());
    let decisao = G5.interpretar(
        &estado,
        Some(&resposta("p-de-outra-org", 0.99)),
        G5.limiares(),
    );
    assert_eq!(decisao.origem, OrigemDecisao::Fallback);
    assert!(decisao.escolha.is_none());
}
