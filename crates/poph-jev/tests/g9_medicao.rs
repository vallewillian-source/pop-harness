use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use poph_jev::{
    BatchResponse, FakeJev, GateEvaluator, GateOverrides, JevEngine, JevFlow, JevMessage,
    JevRequest, JevResponse, JevState, JevTransport,
};
use poph_types::JevGates;

const MAX_TRIGGER_CHARS: usize = 400;

fn fluxo(index: usize) -> JevFlow {
    let filler = "a".repeat(MAX_TRIGGER_CHARS);
    JevFlow {
        id: format!("fluxo_{index}"),
        nome: format!("Fluxo {index}"),
        when: format!("O cliente quer o fluxo {index}. {filler}"),
        not_when: format!("Só está falando de outra coisa. {filler}"),
        examples: vec![
            format!("quero o fluxo {index}"),
            format!("dá para o fluxo {index}?"),
            format!("preciso do fluxo {index}"),
        ],
    }
}

fn estado(round: usize, fluxos: usize) -> JevState {
    let messages = [
        JevMessage::from_customer("Oi, preciso de ajuda com a minha internet."),
        JevMessage::from_agent("Claro, posso ajudar. O que aconteceu?"),
        JevMessage::from_customer(format!("A conexão caiu de novo, é a {round}ª vez hoje.")),
    ];
    let base = JevState::new("whatsapp")
        .with_messages(messages)
        .with_capabilities(true, false);
    if fluxos == 0 {
        base
    } else {
        base.with_released_flows((0..fluxos).map(fluxo))
    }
}

fn fake_engine() -> (Arc<FakeJev>, JevEngine) {
    let fake = Arc::new(FakeJev::new(|request: &JevRequest| {
        let mut replies = BTreeMap::new();
        for (id, question) in &request.questions {
            let choice = question
                .rubric
                .keys()
                .next()
                .cloned()
                .unwrap_or_else(|| "nenhum".to_owned());
            let mut response = JevResponse {
                choice: Some(choice.clone()),
                confidence: 1.0,
                ..JevResponse::default()
            };
            response.probabilities.insert(choice, 1.0);
            replies.insert(id.clone(), response);
        }
        Ok(BatchResponse {
            answers: replies,
            ..BatchResponse::default()
        })
    }));
    let engine = JevEngine::new(
        Arc::clone(&fake) as Arc<dyn JevTransport>,
        JevGates::default_value(),
        "jev-de-medicao",
    );
    (fake, engine)
}

fn percentile(sorted: &[u128], p: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let index = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[index]
}

#[allow(clippy::type_complexity)]
async fn measure(
    rounds: usize,
    fluxos: usize,
    overrides: Option<GateOverrides>,
    label: &str,
) -> (u128, u128, usize, usize, usize) {
    let (fake, engine) = fake_engine();
    let mut samples = Vec::with_capacity(rounds);
    for round in 0..rounds {
        let estado = estado(round, fluxos);
        let started = Instant::now();
        let resultado = engine.evaluate(estado, overrides).await;
        samples.push(started.elapsed().as_micros());
        assert!(!resultado.failed(), "o lote de {label} falhou");
    }
    let requests = fake.requests();
    let last_request = requests.last().expect("o lote chamou o Jev");
    let bytes = serde_json::to_vec(&last_request.to_json())
        .expect("serializa o pedido")
        .len();
    let state_bytes = serde_json::to_vec(&last_request.estado)
        .expect("serializa o estado")
        .len();
    let questions = last_request.questions.len();

    let mut sorted_samples = samples.clone();
    sorted_samples.sort_unstable();
    let p50 = percentile(&sorted_samples, 0.50);
    let p95 = percentile(&sorted_samples, 0.95);
    println!(
        "{label}: {questions} perguntas no lote, p50 {p50} µs, p95 {p95} µs, pedido {bytes} B \
         (estado {state_bytes} B, perguntas {} B)",
        bytes.saturating_sub(state_bytes)
    );
    (p50, p95, bytes, state_bytes, questions)
}

#[tokio::test]
#[ignore = "medição de tempo e tamanho: rodar à mão com --ignored --nocapture"]
async fn g9_cost_in_batch_with_20_flows() {
    const ROUNDS: usize = 200;

    let (_, _, bytes_no_flows, state_no_flows, questions_no_flows) =
        measure(ROUNDS, 0, None, "sem fluxo liberado (G9 inaplicável)").await;
    let (_, _, bytes_twenty, state_twenty, questions_twenty) =
        measure(ROUNDS, 20, None, "20 fluxos, G9 ligado").await;
    let (_, _, bytes_flag_off, state_flag_off, questions_flag_off) = measure(
        ROUNDS,
        20,
        Some(GateOverrides {
            g9: Some(false),
            ..GateOverrides::default()
        }),
        "20 fluxos, G9 desligado pela flag",
    )
    .await;

    println!(
        "estado com 20 fluxos: {state_twenty} B contra {state_no_flows} B sem fluxo liberado (+{} B)",
        state_twenty.saturating_sub(state_no_flows)
    );
    println!(
        "custo do G9 no lote: +{} B de pedido e {} pergunta a mais ({questions_no_flows} → \
         {questions_twenty})",
        bytes_twenty.saturating_sub(bytes_no_flows),
        questions_twenty.saturating_sub(questions_no_flows)
    );
    println!(
        "com a flag desligada: pedido {bytes_flag_off} B (estado {state_flag_off} B, os 20 fluxos vão \
         junto) e {questions_flag_off} perguntas"
    );
}
