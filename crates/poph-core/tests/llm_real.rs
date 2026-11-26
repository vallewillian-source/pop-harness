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
async fn real_call_with_stream_and_tool() {
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
    let metrics = BaseMetrics::register(&Registry::new()).expect("catálogo");
    let client = OpenAiCompatibleClient::new(
        config,
        LlmLimiter::new(1, 1).expect("limites"),
        metrics.clone(),
    )
    .expect("cliente");

    let mut request = Request::new(
        OrgId::generate(),
        vec![
            Message::System("Você é um atendente objetivo. Responda em português do Brasil.".to_owned()),
            Message::User(vec![Part::Texto(
                "Use a ferramenta buscar_produto com o termo 'dedetização de baratas' antes de responder.".to_owned(),
            )]),
        ],
    );
    request.tools = vec![Tool {
        nome: "buscar_produto".to_owned(),
        descricao: "Busca um produto no catálogo e devolve preço e condições".to_owned(),
        parameters: json!({
            "type": "object",
            "properties": {"termo": {"type": "string", "description": "nome do produto"}},
            "required": ["termo"]
        }),
    }];

    let started = Instant::now();
    let mut event_stream = client.stream(request).await.expect("abre o stream");
    let first_item = event_stream.next().await;
    let first_event_ms = started.elapsed().as_millis();

    let mut events = Vec::new();
    if let Some(item) = first_item {
        events.push(item);
    }
    while let Some(item) = event_stream.next().await {
        events.push(item);
    }
    let total_ms = started.elapsed().as_millis();

    let mut text_chunks = 0usize;
    let mut tool_calls = Vec::new();
    let mut usage = None;
    let mut fim = None;
    for item in events {
        match item.expect("evento sem erro") {
            LlmEvent::Texto(_) => text_chunks += 1,
            LlmEvent::Tool(call) => tool_calls.push(call),
            LlmEvent::Usage(u) => usage = Some(u),
            LlmEvent::Fim(motivo) => fim = Some(motivo),
        }
    }

    println!("modelo: {modelo}");
    println!("primeiro evento: {first_event_ms} ms; fim do stream: {total_ms} ms");
    println!("trechos de texto: {text_chunks}");
    println!(
        "chamadas de ferramenta: {} {:?}",
        tool_calls.len(),
        tool_calls
            .iter()
            .map(|c| c.nome.as_str())
            .collect::<Vec<_>>()
    );
    for call in &tool_calls {
        println!("argumentos da ferramenta {}: {}", call.nome, call.arguments);
    }
    println!("uso: {usage:?}");
    println!("motivo do fim: {fim:?}");

    assert!(fim.is_some(), "o stream precisa terminar com Fim");
    assert!(
        !tool_calls.is_empty() || text_chunks > 0,
        "a resposta precisa trazer texto ou chamada de ferramenta"
    );
}
