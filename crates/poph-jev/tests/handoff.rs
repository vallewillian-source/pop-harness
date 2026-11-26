//! Segunda pergunta do G1 (orientação ao cliente antes da transferência) — HR-29.
//!
//! O que se prova aqui: os textos padrão são **exatamente** os aprovados em
//! `HUMAN_HANDOFF_GUIDANCE_PRPD` §5.3; a pergunta de risco sai numa chamada própria (fora do lote
//! do turno); prazo e erro viram "sem orientação" sem derrubar nada; e a composição da mensagem ao
//! cliente é a do POP Atende (orientação + linha em branco + mensagem automática).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use poph_jev::{
    AvaliadorGates, BatchResponse, JevError, JevFalso, JevMessage, JevResponse, JevState, MotorJev,
    Orientacao, RISCO_ANIMAL, RISCO_INGESTAO, RISCO_NENHUM, RISCO_OUTRO, RISCO_PERGUNTA_ID,
    ResultadoOrientacao, TIPOS_DE_RISCO,
};
use poph_types::JevGates;

/// Textos aprovados em 19/11/2026 (`HUMAN_HANDOFF_GUIDANCE_PRD.md` §5.3), copiados do documento.
const APROVADO_INGESTAO: &str = "⚠️ Se alguém engoliu ou teve contato com o produto, procure \
atendimento médico agora ou ligue para o SAMU (192) ou para o Disque-Intoxicação \
(0800 722 6001). Leve a embalagem do produto. Não provoque vômito e não dê nada para beber ou \
comer sem orientação médica.";
const APROVADO_ANIMAL: &str = "⚠️ Não toque nem tente pegar o animal. Mantenha crianças e outras \
pessoas longe dele e, se puder, feche a porta do cômodo. Se alguém foi picado, vá agora a um \
pronto-socorro ou ligue para o SAMU (192).";
const APROVADO_OUTRO: &str = "⚠️ Se alguém corre risco de saúde agora, ligue para o SAMU (192) ou \
vá a um pronto-socorro sem esperar a nossa resposta.";
/// Mensagem automática de hoje do POP Atende (`HUMAN_HANDOFF_AUTO_MESSAGE`).
const APROVADA_AUTOMATICA: &str = "Sua conversa foi transferida para um atendente humano da nossa \
equipe, que já irá respondê-la por aqui. 🙋";

fn estado() -> JevState {
    JevState::new("cli")
        .with_messages([JevMessage::from_customer(
            "meu filho engoliu o veneno, o que eu faço?",
        )])
        .with_handoff(Vec::new(), true)
}

fn motor(jev: Arc<JevFalso>) -> MotorJev {
    MotorJev::novo(
        jev,
        JevGates {
            g1: true,
            ..JevGates::default()
        },
        "jev-latest",
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

/// Jev falso que responde a pergunta de risco com a escolha e a confiança dadas.
fn jev_do_risco(escolha: &'static str, confianca: f64) -> Arc<JevFalso> {
    Arc::new(JevFalso::novo(move |pedido: &poph_jev::Pedido| {
        let mut respostas = BTreeMap::new();
        if pedido.perguntas.contains_key(RISCO_PERGUNTA_ID) {
            respostas.insert(RISCO_PERGUNTA_ID.to_owned(), resposta(escolha, confianca));
        }
        Ok(BatchResponse {
            answers: respostas,
            tokens: 7,
            ..BatchResponse::default()
        })
    }))
}

// ───────────────────────────── Textos aprovados ─────────────────────────────

/// Aceite 2: os três textos e a mensagem automática são, byte a byte, os aprovados.
#[test]
fn textos_padrao_sao_exatamente_os_aprovados() {
    let orientacao = Orientacao::default();
    assert_eq!(orientacao.texto(RISCO_INGESTAO), APROVADO_INGESTAO);
    assert_eq!(orientacao.texto(RISCO_ANIMAL), APROVADO_ANIMAL);
    assert_eq!(orientacao.texto(RISCO_OUTRO), APROVADO_OUTRO);
    assert_eq!(orientacao.mensagem_automatica, APROVADA_AUTOMATICA);
    // Padrão do POP Atende: orientação ligada, limiar 0,5 e 4 s só nesta pergunta.
    assert!(orientacao.ativo);
    assert!((orientacao.confianca - 0.5).abs() < f64::EPSILON);
    assert_eq!(orientacao.prazo, Duration::from_secs(4));
}

/// A mensagem ao cliente é orientação + linha em branco + automática; sem orientação, só a
/// automática (comportamento de hoje, byte a byte).
#[test]
fn compoe_orientacao_e_mensagem_automatica() {
    let orientacao = Orientacao::default();
    assert_eq!(
        orientacao.compor(Some(RISCO_ANIMAL)),
        format!("{APROVADO_ANIMAL}\n\n{APROVADA_AUTOMATICA}")
    );
    assert_eq!(orientacao.compor(None), APROVADA_AUTOMATICA);
    assert_eq!(orientacao.compor(Some(RISCO_NENHUM)), APROVADA_AUTOMATICA);
    assert_eq!(
        orientacao.compor(Some("tipo-desconhecido")),
        APROVADA_AUTOMATICA
    );
    // Texto vazio conta como vazio (§5.3): o tipo não orienta.
    let sem_texto = Orientacao::default().com_texto(RISCO_OUTRO, "   ");
    assert_eq!(sem_texto.compor(Some(RISCO_OUTRO)), APROVADA_AUTOMATICA);
}

// ───────────────────────────── Pergunta e distribuição ─────────────────────────────

/// A pergunta de risco é própria ("risco_imediato") e não entra no lote dos gates.
#[tokio::test]
async fn pergunta_de_risco_nao_entra_no_lote() {
    let jev = jev_do_risco(RISCO_INGESTAO, 0.92);
    let avaliador = motor(Arc::clone(&jev));

    let lote = avaliador.avaliar(estado(), None).await;
    let pedidos = jev.pedidos();
    assert_eq!(pedidos.len(), 1, "o lote é uma chamada só");
    assert!(pedidos[0].perguntas.contains_key("g1_transferir"));
    assert!(
        !pedidos[0].perguntas.contains_key(RISCO_PERGUNTA_ID),
        "a pergunta de risco não pode ir no lote do turno"
    );
    // Só G1 ligado e sem resposta roteirizada para ele: o gate absteve.
    assert!(lote.falhou() || lote.decisao(poph_jev::IdGate::G1).is_some());

    let risco = avaliador.orientar(estado(), Duration::from_secs(1)).await;
    assert!(risco.consultado);
    assert_eq!(risco.tipo.as_deref(), Some(RISCO_INGESTAO));
    assert!((risco.confianca - 0.92).abs() < 1e-9);
    assert_eq!(risco.tokens, 7);
    let pedidos = jev.pedidos();
    assert_eq!(pedidos.len(), 2, "a segunda pergunta é uma chamada própria");
    assert_eq!(
        pedidos[1].perguntas.keys().collect::<Vec<_>>(),
        vec![RISCO_PERGUNTA_ID]
    );
}

/// A pergunta carrega as quatro opções do contrato do POP Atende.
#[test]
fn pergunta_tem_as_quatro_opcoes() {
    let pergunta = poph_jev::pergunta_risco();
    let opcoes: Vec<&String> = pergunta.rubric.keys().collect();
    assert_eq!(opcoes.len(), 4);
    for tipo in [RISCO_NENHUM, RISCO_INGESTAO, RISCO_ANIMAL, RISCO_OUTRO] {
        assert!(pergunta.rubric.contains_key(tipo), "falta a opção {tipo}");
    }
    assert!(!pergunta.rubric[RISCO_NENHUM].trim().is_empty());
    assert_eq!(TIPOS_DE_RISCO.len(), 3);
}

// ───────────────────────────── Fail-safe ─────────────────────────────

/// Prazo: a pergunta é abortada e o resultado volta como falha — a transferência não espera.
#[tokio::test]
async fn prazo_da_segunda_pergunta_vira_falha() {
    // O atraso atinge as duas chamadas; o prazo da segunda (40 ms) é o que se mede aqui.
    let jev = Arc::new(
        JevFalso::com_escolhas(BTreeMap::from([(
            RISCO_PERGUNTA_ID.to_owned(),
            RISCO_INGESTAO.to_owned(),
        )]))
        .com_atraso(Duration::from_millis(300)),
    );
    let avaliador = motor(Arc::clone(&jev));
    let inicio = std::time::Instant::now();
    let risco = avaliador
        .orientar(estado(), Duration::from_millis(40))
        .await;
    let decorrido = inicio.elapsed();

    assert!(risco.consultado);
    assert!(risco.falhou(), "prazo estourado é falha, não tipo");
    assert!(risco.tipo.is_none());
    assert!(
        decorrido < Duration::from_millis(250),
        "a pergunta tem de desistir no próprio prazo, levou {decorrido:?}"
    );
}

/// Erro do provedor também vira falha, nunca pânico nem tipo inventado.
#[tokio::test]
async fn erro_do_provedor_vira_falha() {
    let jev = Arc::new(JevFalso::falhando(JevError::Status {
        code: 503,
        motivo: "Service Unavailable".to_owned(),
    }));
    let avaliador = motor(jev);
    let risco = avaliador.orientar(estado(), Duration::from_secs(1)).await;
    assert!(risco.falhou());
    assert!(risco.tipo.is_none());
}

/// Toda forma de "sem decisão confiável" suprime a orientação.
#[tokio::test]
async fn sem_decisao_confiavel_nao_orienta() {
    let orientacao = Orientacao::default();

    // `nenhum`: a conversa não tem risco imediato.
    let nenhum = jev_do_risco(RISCO_NENHUM, 0.99);
    let risco = motor(Arc::clone(&nenhum))
        .orientar(estado(), Duration::from_secs(1))
        .await;
    assert_eq!(orientacao.tipo_confiavel(&risco), None);

    // Escolha fora do contrato.
    let fora = jev_do_risco("risco_qualquer", 0.99);
    let risco = motor(Arc::clone(&fora))
        .orientar(estado(), Duration::from_secs(1))
        .await;
    assert_eq!(orientacao.tipo_confiavel(&risco), None);

    // Confiança abaixo do limiar (0,5).
    let baixa = jev_do_risco(RISCO_INGESTAO, 0.49);
    let risco = motor(Arc::clone(&baixa))
        .orientar(estado(), Duration::from_secs(1))
        .await;
    assert_eq!(orientacao.tipo_confiavel(&risco), None);

    // Passa do limiar: orienta.
    let confiavel = jev_do_risco(RISCO_INGESTAO, 0.5);
    let risco = motor(Arc::clone(&confiavel))
        .orientar(estado(), Duration::from_secs(1))
        .await;
    assert_eq!(
        orientacao.tipo_confiavel(&risco),
        Some(RISCO_INGESTAO.to_owned())
    );

    // Orientação desligada não orienta nem com risco confiável.
    let desligada = Orientacao::default();
    let desligada = Orientacao {
        ativo: false,
        ..desligada
    };
    assert_eq!(desligada.tipo_confiavel(&risco), None);

    // Resultado não consultado (Jev desligado) nunca orienta.
    let nao_consultado = ResultadoOrientacao::nao_consultado("Jev desligado");
    assert_eq!(orientacao.tipo_confiavel(&nao_consultado), None);
}
