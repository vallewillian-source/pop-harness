//! Teste manual com o gateway real (antigravity2api em 127.0.0.1:8045). Fica fora do CI.
//!
//! Uso (a chave vem só do ambiente; o teste não a imprime):
//!   ANTIGRAVITY_API_KEY=... cargo test -p poph-core --test llm_real -- --ignored --nocapture
//!
//! Faz uma única chamada em stream, com uma ferramenta oferecida e uma instrução que pede o uso
//! dela. Imprime só tempos, tipos de evento e uso de tokens.

use std::time::{Duration, Instant};

use futures::StreamExt;
use poph_core::llm::{
    LlmEvent, LlmLimiter, LlmProvider, Message, OpenAiCompatibleClient, OpenAiCompatibleConfig,
    Part, Request, Tool,
};
use poph_types::{BaseMetrics, OrgId, Registry, Secret};
use serde_json::json;

#[tokio::test]
#[ignore = "chamada real ao gateway; roda só com ANTIGRAVITY_API_KEY definida"]
async fn uma_chamada_real_com_stream_e_ferramenta() {
    let chave =
        std::env::var("ANTIGRAVITY_API_KEY").expect("defina ANTIGRAVITY_API_KEY no ambiente");
    let url_base = std::env::var("ANTIGRAVITY_BASE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8045/v1".to_owned());
    let modelo =
        std::env::var("ANTIGRAVITY_MODEL").unwrap_or_else(|_| "gemini-3.8-flash-tiered".to_owned());

    let config = OpenAiCompatibleConfig {
        url_base,
        chave: Secret::new(chave),
        modelo: modelo.clone(),
        timeout: Duration::from_secs(120),
        retentativas: 0,
        base_wait: Duration::from_secs(1),
        temperatura: 0.0,
        max_tokens_saida: 512,
        recusa_turno_modelo: true,
    };
    let metricas = BaseMetrics::register(&Registry::new()).expect("catálogo");
    let cliente = OpenAiCompatibleClient::new(
        config,
        LlmLimiter::new(1, 1).expect("limites"),
        metricas.clone(),
    )
    .expect("cliente");

    let mut pedido = Request::new(
        OrgId::generate(),
        vec![
            Message::System("Você é um atendente objetivo. Responda em português do Brasil.".to_owned()),
            Message::User(vec![Part::Texto(
                "Use a ferramenta buscar_produto com o termo 'dedetização de baratas' antes de responder.".to_owned(),
            )]),
        ],
    );
    pedido.tools = vec![Tool {
        nome: "buscar_produto".to_owned(),
        descricao: "Busca um produto no catálogo e devolve preço e condições".to_owned(),
        parameters: json!({
            "type": "object",
            "properties": {"termo": {"type": "string", "description": "nome do produto"}},
            "required": ["termo"]
        }),
    }];

    let inicio = Instant::now();
    let mut fluxo = cliente.stream(pedido).await.expect("abre o stream");
    let primeiro = fluxo.next().await;
    let primeiro_evento_ms = inicio.elapsed().as_millis();

    let mut eventos = Vec::new();
    if let Some(item) = primeiro {
        eventos.push(item);
    }
    while let Some(item) = fluxo.next().await {
        eventos.push(item);
    }
    let total_ms = inicio.elapsed().as_millis();

    let mut textos = 0usize;
    let mut ferramentas = Vec::new();
    let mut uso = None;
    let mut fim = None;
    for item in eventos {
        match item.expect("evento sem erro") {
            LlmEvent::Texto(_) => textos += 1,
            LlmEvent::Tool(chamada) => ferramentas.push(chamada),
            LlmEvent::Usage(u) => uso = Some(u),
            LlmEvent::Fim(motivo) => fim = Some(motivo),
        }
    }

    println!("modelo: {modelo}");
    println!("primeiro evento: {primeiro_evento_ms} ms; fim do stream: {total_ms} ms");
    println!("trechos de texto: {textos}");
    println!(
        "chamadas de ferramenta: {} {:?}",
        ferramentas.len(),
        ferramentas
            .iter()
            .map(|c| c.nome.as_str())
            .collect::<Vec<_>>()
    );
    for chamada in &ferramentas {
        println!(
            "argumentos da ferramenta {}: {}",
            chamada.nome, chamada.arguments
        );
    }
    println!("uso: {uso:?}");
    println!("motivo do fim: {fim:?}");

    assert!(fim.is_some(), "o stream precisa terminar com Fim");
    assert!(
        !ferramentas.is_empty() || textos > 0,
        "a resposta precisa trazer texto ou chamada de ferramenta"
    );
}
