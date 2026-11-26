use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use poph_jev::{
    ANIMAL_RISK, BatchResponse, FakeJev, GateEvaluator, GuidanceResult, HandoffGuidance,
    INGESTION_RISK, JevEngine, JevError, JevMessage, JevResponse, JevState, NO_RISK, OTHER_RISK,
    RISK_QUESTION_ID, RISK_TYPES,
};
use poph_types::JevGates;

const APPROVED_INGESTION: &str = "⚠️ Se alguém engoliu ou teve contato com o produto, procure \
atendimento médico agora ou ligue para o SAMU (192) ou para o Disque-Intoxicação \
(0800 722 6001). Leve a embalagem do produto. Não provoque vômito e não dê nada para beber ou \
comer sem orientação médica.";
const APPROVED_ANIMAL: &str = "⚠️ Não toque nem tente pegar o animal. Mantenha crianças e outras \
pessoas longe dele e, se puder, feche a porta do cômodo. Se alguém foi picado, vá agora a um \
pronto-socorro ou ligue para o SAMU (192).";
const APPROVED_OTHER: &str = "⚠️ Se alguém corre risco de saúde agora, ligue para o SAMU (192) ou \
vá a um pronto-socorro sem esperar a nossa resposta.";
const APPROVED_AUTO_MESSAGE: &str = "Sua conversa foi transferida para um atendente humano da nossa \
equipe, que já irá respondê-la por aqui. 🙋";

fn estado() -> JevState {
    JevState::new("cli")
        .with_messages([JevMessage::from_customer(
            "meu filho engoliu o veneno, o que eu faço?",
        )])
        .with_handoff(Vec::new(), true)
}

fn engine(jev: Arc<FakeJev>) -> JevEngine {
    JevEngine::new(
        jev,
        JevGates {
            g1: true,
            ..JevGates::default()
        },
        "jev-latest",
    )
}

fn answer_of(choice: &str, certainty: f64) -> JevResponse {
    let mut item = JevResponse {
        choice: Some(choice.to_owned()),
        confidence: certainty,
        ..JevResponse::default()
    };
    item.probabilities.insert(choice.to_owned(), certainty);
    item
}

fn risk_jev(choice: &'static str, certainty: f64) -> Arc<FakeJev> {
    Arc::new(FakeJev::new(move |request: &poph_jev::JevRequest| {
        let mut answer_map = BTreeMap::new();
        if request.questions.contains_key(RISK_QUESTION_ID) {
            answer_map.insert(RISK_QUESTION_ID.to_owned(), answer_of(choice, certainty));
        }
        Ok(BatchResponse {
            answers: answer_map,
            tokens: 7,
            ..BatchResponse::default()
        })
    }))
}

#[test]
fn default_texts_are_exactly_the_approved_ones() {
    let guidance = HandoffGuidance::default();
    assert_eq!(guidance.texto(INGESTION_RISK), APPROVED_INGESTION);
    assert_eq!(guidance.texto(ANIMAL_RISK), APPROVED_ANIMAL);
    assert_eq!(guidance.texto(OTHER_RISK), APPROVED_OTHER);
    assert_eq!(guidance.mensagem_automatica, APPROVED_AUTO_MESSAGE);
    assert!(guidance.ativo);
    assert!((guidance.confidence - 0.5).abs() < f64::EPSILON);
    assert_eq!(guidance.timeout, Duration::from_secs(4));
}

#[test]
fn composes_guidance_and_auto_message() {
    let guidance = HandoffGuidance::default();
    assert_eq!(
        guidance.compose(Some(ANIMAL_RISK)),
        format!("{APPROVED_ANIMAL}\n\n{APPROVED_AUTO_MESSAGE}")
    );
    assert_eq!(guidance.compose(None), APPROVED_AUTO_MESSAGE);
    assert_eq!(guidance.compose(Some(NO_RISK)), APPROVED_AUTO_MESSAGE);
    assert_eq!(
        guidance.compose(Some("tipo-desconhecido")),
        APPROVED_AUTO_MESSAGE
    );
    let without_text = HandoffGuidance::default().with_text(OTHER_RISK, "   ");
    assert_eq!(
        without_text.compose(Some(OTHER_RISK)),
        APPROVED_AUTO_MESSAGE
    );
}

#[tokio::test]
async fn risk_question_stays_out_of_the_batch() {
    let jev = risk_jev(INGESTION_RISK, 0.92);
    let evaluator = engine(Arc::clone(&jev));

    let batch = evaluator.evaluate(estado(), None).await;
    let requests = jev.requests();
    assert_eq!(requests.len(), 1, "o lote é uma chamada só");
    assert!(requests[0].questions.contains_key("g1_transferir"));
    assert!(
        !requests[0].questions.contains_key(RISK_QUESTION_ID),
        "a pergunta de risco não pode ir no lote do turno"
    );
    assert!(batch.failed() || batch.decision(poph_jev::IdGate::G1).is_some());

    let risk = evaluator.orientar(estado(), Duration::from_secs(1)).await;
    assert!(risk.queried);
    assert_eq!(risk.tipo.as_deref(), Some(INGESTION_RISK));
    assert!((risk.confidence - 0.92).abs() < 1e-9);
    assert_eq!(risk.tokens, 7);
    let requests = jev.requests();
    assert_eq!(
        requests.len(),
        2,
        "a segunda pergunta é uma chamada própria"
    );
    assert_eq!(
        requests[1].questions.keys().collect::<Vec<_>>(),
        vec![RISK_QUESTION_ID]
    );
}

#[test]
fn question_has_the_four_options() {
    let question = poph_jev::risk_question();
    let options: Vec<&String> = question.rubric.keys().collect();
    assert_eq!(options.len(), 4);
    for tipo in [NO_RISK, INGESTION_RISK, ANIMAL_RISK, OTHER_RISK] {
        assert!(question.rubric.contains_key(tipo), "falta a opção {tipo}");
    }
    assert!(!question.rubric[NO_RISK].trim().is_empty());
    assert_eq!(RISK_TYPES.len(), 3);
}

#[tokio::test]
async fn second_question_deadline_becomes_failure() {
    let jev = Arc::new(
        FakeJev::with_choices(BTreeMap::from([(
            RISK_QUESTION_ID.to_owned(),
            INGESTION_RISK.to_owned(),
        )]))
        .with_delay(Duration::from_millis(300)),
    );
    let evaluator = engine(Arc::clone(&jev));
    let start = std::time::Instant::now();
    let risk = evaluator
        .orientar(estado(), Duration::from_millis(40))
        .await;
    let elapsed = start.elapsed();

    assert!(risk.queried);
    assert!(risk.failed(), "prazo estourado é falha, não tipo");
    assert!(risk.tipo.is_none());
    assert!(
        elapsed < Duration::from_millis(250),
        "a pergunta tem de desistir no próprio prazo, levou {elapsed:?}"
    );
}

#[tokio::test]
async fn provider_error_becomes_failure() {
    let jev = Arc::new(FakeJev::failing(JevError::Status {
        code: 503,
        motivo: "Service Unavailable".to_owned(),
    }));
    let evaluator = engine(jev);
    let risk = evaluator.orientar(estado(), Duration::from_secs(1)).await;
    assert!(risk.failed());
    assert!(risk.tipo.is_none());
}

#[tokio::test]
async fn without_reliable_decision_gives_no_guidance() {
    let guidance = HandoffGuidance::default();

    let none = risk_jev(NO_RISK, 0.99);
    let risk = engine(Arc::clone(&none))
        .orientar(estado(), Duration::from_secs(1))
        .await;
    assert_eq!(guidance.reliable_type(&risk), None);

    let outside = risk_jev("risco_qualquer", 0.99);
    let risk = engine(Arc::clone(&outside))
        .orientar(estado(), Duration::from_secs(1))
        .await;
    assert_eq!(guidance.reliable_type(&risk), None);

    let low = risk_jev(INGESTION_RISK, 0.49);
    let risk = engine(Arc::clone(&low))
        .orientar(estado(), Duration::from_secs(1))
        .await;
    assert_eq!(guidance.reliable_type(&risk), None);

    let reliable = risk_jev(INGESTION_RISK, 0.5);
    let risk = engine(Arc::clone(&reliable))
        .orientar(estado(), Duration::from_secs(1))
        .await;
    assert_eq!(
        guidance.reliable_type(&risk),
        Some(INGESTION_RISK.to_owned())
    );

    let disabled = HandoffGuidance::default();
    let disabled = HandoffGuidance {
        ativo: false,
        ..disabled
    };
    assert_eq!(disabled.reliable_type(&risk), None);

    let not_queried = GuidanceResult::not_queried("Jev desligado");
    assert_eq!(guidance.reliable_type(&not_queried), None);
}
