//! Custo do G9 no lote do Jev (FX-36): tempo de montagem e tamanho do pedido com 20 fluxos.
//!
//! O que se mede aqui é o que o Harness gasta **antes** da chamada: montar o estado, montar as
//! perguntas e serializar o pedido. O transporte é o [`JevFalso`], então o tempo da resposta do
//! provedor não entra — essa parte é a medição com o Jev real (FX-37).
//!
//! Rodar assim (medição, não regressão):
//!
//! ```text
//! cargo test --release -p poph-jev --test g9_medicao -- --ignored --nocapture
//! ```
//!
//! Três colunas, para o custo ser atribuível:
//!
//! - **sem fluxo liberado**: o G9 não se aplica; é o lote de antes do módulo;
//! - **20 fluxos, G9 ligado**: o caso de topo (o teto de fluxos por agente é 20, FX-08);
//! - **20 fluxos, G9 desligado pela flag**: o estado cresce, mas a pergunta não existe.
//!
//! O texto de cada gatilho vai no tamanho máximo que a pergunta aceita (400 caracteres, PRD §6.3),
//! porque é o pior caso: um agente com 20 fluxos e textos longos.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use poph_jev::{
    BatchResponse, GateEvaluator, GateOverrides, JevEngine, JevFalso, JevFlow, JevMessage,
    JevResponse, JevState, Pedido, TransporteJev,
};
use poph_types::JevGates;

/// Teto de caracteres do texto de gatilho na pergunta (PRD §6.3).
const MAX_GATILHO: usize = 400;

fn fluxo(indice: usize) -> JevFlow {
    let recheio = "a".repeat(MAX_GATILHO);
    JevFlow {
        id: format!("fluxo_{indice}"),
        nome: format!("Fluxo {indice}"),
        when: format!("O cliente quer o fluxo {indice}. {recheio}"),
        not_when: format!("Só está falando de outra coisa. {recheio}"),
        examples: vec![
            format!("quero o fluxo {indice}"),
            format!("dá para o fluxo {indice}?"),
            format!("preciso do fluxo {indice}"),
        ],
    }
}

/// Estado de um turno com conversa em andamento: base disponível e uma resposta do agente (assim o
/// G3 e o G8 também perguntam, e o lote não é só o G9), sem fluxo liberado.
fn estado(rodada: usize, fluxos: usize) -> JevState {
    let mensagens = [
        JevMessage::from_customer("Oi, preciso de ajuda com a minha internet."),
        JevMessage::from_agent("Claro, posso ajudar. O que aconteceu?"),
        JevMessage::from_customer(format!("A conexão caiu de novo, é a {rodada}ª vez hoje.")),
    ];
    let base = JevState::new("whatsapp")
        .with_messages(mensagens)
        .with_capabilities(true, false);
    if fluxos == 0 {
        base
    } else {
        base.with_released_flows((0..fluxos).map(fluxo))
    }
}

/// Motor com Jev falso: responde a primeira opção de cada pergunta, confiança 1,0.
fn motor() -> (Arc<JevFalso>, JevEngine) {
    let falso = Arc::new(JevFalso::novo(|pedido: &Pedido| {
        let mut respostas = BTreeMap::new();
        for (id, pergunta) in &pedido.perguntas {
            let escolha = pergunta
                .rubric
                .keys()
                .next()
                .cloned()
                .unwrap_or_else(|| "nenhum".to_owned());
            let mut resposta = JevResponse {
                choice: Some(escolha.clone()),
                confidence: 1.0,
                ..JevResponse::default()
            };
            resposta.probabilities.insert(escolha, 1.0);
            respostas.insert(id.clone(), resposta);
        }
        Ok(BatchResponse {
            answers: respostas,
            ..BatchResponse::default()
        })
    }));
    let motor = JevEngine::new(
        Arc::clone(&falso) as Arc<dyn TransporteJev>,
        JevGates::default_value(),
        "jev-de-medicao",
    );
    (falso, motor)
}

fn percentil(ordenadas: &[u128], p: f64) -> u128 {
    if ordenadas.is_empty() {
        return 0;
    }
    let indice = ((ordenadas.len() as f64 - 1.0) * p).round() as usize;
    ordenadas[indice]
}

/// Uma coluna: `rodadas` lotes, opcionalmente com o G9 desligado pela flag.
#[allow(clippy::type_complexity)]
async fn medir(
    rodadas: usize,
    fluxos: usize,
    ajuste: Option<GateOverrides>,
    rotulo: &str,
) -> (u128, u128, usize, usize, usize) {
    let (falso, motor) = motor();
    let mut amostras = Vec::with_capacity(rodadas);
    for rodada in 0..rodadas {
        // O texto muda a cada rodada para o cache de decisão não atender o lote.
        let estado = estado(rodada, fluxos);
        let inicio = Instant::now();
        let resultado = motor.evaluate(estado, ajuste).await;
        amostras.push(inicio.elapsed().as_micros());
        assert!(!resultado.failed(), "o lote de {rotulo} falhou");
    }
    let pedidos = falso.pedidos();
    let ultimo = pedidos.last().expect("o lote chamou o Jev");
    let bytes = serde_json::to_vec(&ultimo.para_json())
        .expect("serializa o pedido")
        .len();
    let bytes_estado = serde_json::to_vec(&ultimo.estado)
        .expect("serializa o estado")
        .len();
    let perguntas = ultimo.perguntas.len();

    let mut ordenadas = amostras.clone();
    ordenadas.sort_unstable();
    let p50 = percentil(&ordenadas, 0.50);
    let p95 = percentil(&ordenadas, 0.95);
    println!(
        "{rotulo}: {perguntas} perguntas no lote, p50 {p50} µs, p95 {p95} µs, pedido {bytes} B \
         (estado {bytes_estado} B, perguntas {} B)",
        bytes.saturating_sub(bytes_estado)
    );
    (p50, p95, bytes, bytes_estado, perguntas)
}

#[tokio::test]
#[ignore = "medição de tempo e tamanho: rodar à mão com --ignored --nocapture"]
async fn custo_do_g9_no_lote_com_20_fluxos() {
    const RODADAS: usize = 200;

    let (_, _, bytes_sem, estado_sem, perguntas_sem) =
        medir(RODADAS, 0, None, "sem fluxo liberado (G9 inaplicável)").await;
    let (_, _, bytes_vinte, estado_vinte, perguntas_vinte) =
        medir(RODADAS, 20, None, "20 fluxos, G9 ligado").await;
    let (_, _, bytes_flag, estado_flag, perguntas_flag) = medir(
        RODADAS,
        20,
        Some(GateOverrides {
            g9: Some(false),
            ..GateOverrides::default()
        }),
        "20 fluxos, G9 desligado pela flag",
    )
    .await;

    println!(
        "estado com 20 fluxos: {estado_vinte} B contra {estado_sem} B sem fluxo liberado (+{} B)",
        estado_vinte.saturating_sub(estado_sem)
    );
    println!(
        "custo do G9 no lote: +{} B de pedido e {} pergunta a mais ({perguntas_sem} → \
         {perguntas_vinte})",
        bytes_vinte.saturating_sub(bytes_sem),
        perguntas_vinte.saturating_sub(perguntas_sem)
    );
    println!(
        "com a flag desligada: pedido {bytes_flag} B (estado {estado_flag} B, os 20 fluxos vão \
         junto) e {perguntas_flag} perguntas"
    );
}
