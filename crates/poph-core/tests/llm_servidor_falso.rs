//! Cliente OpenAI-compatível contra um servidor HTTP falso local (sem rede externa).
//!
//! Cobre: texto em stream, chamada de ferramenta (ida e volta), imagem, 429 e 5xx com
//! retentativa, timeout, cancelamento no meio do stream, erro definitivo, resposta cortada,
//! pedido recusado antes do envio. O teste de logs e chave fica em `llm_logs.rs`.

mod apoio;

use std::time::Duration;

use apoio::*;
use futures::StreamExt;
use poph_core::llm::base64::encode;
use poph_core::llm::{
    FinishReason, LlmError, LlmEvent, LlmProvider, Message, Part, Request, Tool, ToolCall, Usage,
};
use poph_types::OrgId;
use serde_json::json;

const TEXTO_OLA: &str = r#"{"choices":[{"index":0,"delta":{"role":"assistant","content":"Olá"},"finish_reason":null}]}"#;
const TEXTO_FIM: &str =
    r#"{"choices":[{"index":0,"delta":{"content":", tudo bem?"},"finish_reason":null}]}"#;
const FIM_COM_USO: &str = r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":110,"completion_tokens":7,"total_tokens":117}}"#;

#[tokio::test]
async fn texto_chega_em_stream_com_uso_e_fim() {
    let servidor = Servidor::iniciar(vec![Resposta::ok(sse(&[
        TEXTO_OLA,
        TEXTO_FIM,
        FIM_COM_USO,
        "[DONE]",
    ]))])
    .await;
    let m = montar(&servidor.url_base, 2, Duration::from_secs(5));

    let fluxo = m.cliente.stream(pergunta("oi")).await.expect("abre");
    let eventos = so_eventos(colher(fluxo).await);

    assert_eq!(
        eventos,
        vec![
            LlmEvent::Texto("Olá".into()),
            LlmEvent::Texto(", tudo bem?".into()),
            LlmEvent::Usage(Usage {
                entrada: 110,
                saida: 7
            }),
            LlmEvent::Fim(FinishReason::Concluido),
        ]
    );
    let corpo = &servidor.pedidos()[0];
    assert_eq!(corpo["stream"], json!(true));
    assert_eq!(corpo["model"], "gemini-3.8-flash-tiered");
    let mensagens = corpo["messages"].as_array().expect("lista");
    assert_eq!(mensagens.last().expect("última")["role"], "user");
    assert_eq!(m.metricas.tokens_total.value(), 117);
    assert_eq!(m.metricas.llm_calls.value(), 1);
}

#[tokio::test]
async fn chamada_de_ferramenta_sai_com_argumentos_inteiros_e_volta_com_resultado() {
    let chamada_1 = sse(&[
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"buscar_produto","arguments":"{\"termo\":"}}]}}]}"#,
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"dedetização\"}"}}]}}]}"#,
        r#"{"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
        "[DONE]",
    ]);
    let chamada_2 = sse(&[
        r#"{"choices":[{"index":0,"delta":{"content":"O preço é R$ 350."},"finish_reason":"stop"}]}"#,
        "[DONE]",
    ]);
    let servidor = Servidor::iniciar(vec![Resposta::ok(chamada_1), Resposta::ok(chamada_2)]).await;
    let m = montar(&servidor.url_base, 2, Duration::from_secs(5));
    let org = OrgId::generate();

    let mut primeiro = Request::new(
        org,
        vec![Message::Usuario(vec![Part::Texto("quanto custa?".into())])],
    );
    primeiro.tools = vec![Tool {
        nome: "buscar_produto".into(),
        descricao: "Busca um produto e devolve o preço".into(),
        parametros: json!({"type": "object", "properties": {"termo": {"type": "string"}}}),
    }];
    let fluxo = m.cliente.stream(primeiro).await.expect("abre");
    let eventos = so_eventos(colher(fluxo).await);
    assert_eq!(
        eventos,
        vec![
            LlmEvent::Tool(ToolCall {
                id: "call_1".into(),
                nome: "buscar_produto".into(),
                arguments: json!({"termo": "dedetização"}),
            }),
            LlmEvent::Fim(FinishReason::ChamadasFerramenta),
        ]
    );
    let corpo_1 = &servidor.pedidos()[0];
    assert_eq!(corpo_1["tools"][0]["function"]["name"], "buscar_produto");

    // Volta com o resultado: a última mensagem é do resultado, e a chamada aparece antes dele.
    let segundo = pedido_com(vec![
        Message::Usuario(vec![Part::Texto("quanto custa?".into())]),
        Message::Assistente {
            texto: String::new(),
            chamadas: vec![ToolCall {
                id: "call_1".into(),
                nome: "buscar_produto".into(),
                arguments: json!({"termo": "dedetização"}),
            }],
        },
        Message::Resultado {
            id: "call_1".into(),
            conteudo: "R$ 350,00".into(),
        },
    ]);
    let fluxo = m.cliente.stream(segundo).await.expect("abre");
    let eventos = so_eventos(colher(fluxo).await);
    assert_eq!(
        eventos,
        vec![
            LlmEvent::Texto("O preço é R$ 350.".into()),
            LlmEvent::Fim(FinishReason::Concluido),
        ]
    );
    let corpo_2 = &servidor.pedidos()[1];
    let mensagens = corpo_2["messages"].as_array().expect("lista");
    assert_eq!(mensagens[1]["role"], "assistant");
    assert_eq!(mensagens[1]["tool_calls"][0]["id"], "call_1");
    assert_eq!(
        mensagens[1]["tool_calls"][0]["function"]["arguments"],
        "{\"termo\":\"dedetização\"}"
    );
    assert_eq!(mensagens[2]["role"], "tool");
    assert_eq!(mensagens[2]["tool_call_id"], "call_1");
    assert_eq!(mensagens[2]["content"], "R$ 350,00");
}

#[tokio::test]
async fn imagem_vai_como_data_url_junto_do_texto() {
    let servidor = Servidor::iniciar(vec![Resposta::ok(sse(&[
        r#"{"choices":[{"index":0,"delta":{"content":"ARARA AZUL 42"},"finish_reason":"stop"}]}"#,
        "[DONE]",
    ]))])
    .await;
    let m = montar(&servidor.url_base, 0, Duration::from_secs(5));
    let bytes = b"\x89PNG\r\n\x1a\nconteudo".to_vec();
    let pedido = pedido_com(vec![Message::Usuario(vec![
        Part::Texto("o que está escrito?".into()),
        Part::Imagem {
            mime: "image/png".into(),
            bytes: bytes.clone(),
        },
    ])]);

    let fluxo = m.cliente.stream(pedido).await.expect("abre");
    let eventos = so_eventos(colher(fluxo).await);
    assert_eq!(eventos[0], LlmEvent::Texto("ARARA AZUL 42".into()));

    let corpo = &servidor.pedidos()[0];
    let partes = corpo["messages"][0]["content"].as_array().expect("partes");
    assert_eq!(partes[0]["type"], "text");
    assert_eq!(partes[1]["type"], "image_url");
    assert_eq!(
        partes[1]["image_url"]["url"],
        format!("data:image/png;base64,{}", encode(&bytes))
    );
}

#[tokio::test]
async fn retenta_429_e_5xx_sem_mudar_o_pedido() {
    let servidor = Servidor::iniciar(vec![
        Resposta::erro(429, r#"{"error":{"message":"muitas requisições"}}"#),
        Resposta::erro(503, "indisponível"),
        Resposta::ok(sse(&[
            r#"{"choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":"stop"}]}"#,
            "[DONE]",
        ])),
    ])
    .await;
    let m = montar(&servidor.url_base, 2, Duration::from_secs(5));

    let fluxo = m
        .cliente
        .stream(pergunta("oi"))
        .await
        .expect("3ª tentativa dá certo");
    let eventos = so_eventos(colher(fluxo).await);
    assert_eq!(eventos[0], LlmEvent::Texto("ok".into()));

    let pedidos = servidor.pedidos();
    assert_eq!(pedidos.len(), 3);
    assert_eq!(pedidos[0], pedidos[1]);
    assert_eq!(pedidos[1], pedidos[2]);
}

#[tokio::test]
async fn erro_definitivo_nao_e_retentado_e_nao_ecoa_a_chave() {
    let corpo = format!(r#"{{"error":{{"message":"chave recusada: {CHAVE}"}}}}"#);
    let servidor = Servidor::iniciar(vec![Resposta::erro(401, &corpo)]).await;
    let m = montar(&servidor.url_base, 2, Duration::from_secs(5));

    let erro = m
        .cliente
        .stream(pergunta("oi"))
        .await
        .err()
        .expect("recusado");
    match &erro {
        LlmError::Rejected { status, mensagem } => {
            assert_eq!(*status, 401);
            assert!(!mensagem.contains(CHAVE));
        }
        outro => panic!("esperava Recusado, veio {outro:?}"),
    }
    assert!(!format!("{erro} {erro:?}").contains(CHAVE));
    assert_eq!(servidor.pedidos().len(), 1, "401 não pode ser retentado");
    assert_eq!(m.metricas.llm_errors.value(), 1);
}

#[tokio::test]
async fn esgotar_retentativas_devolve_indisponivel() {
    let servidor = Servidor::iniciar(vec![Resposta::erro(500, "falhou")]).await;
    let m = montar(&servidor.url_base, 2, Duration::from_secs(5));

    let erro = m.cliente.stream(pergunta("oi")).await.err().expect("falha");
    assert_eq!(
        erro,
        LlmError::Unavailable {
            tentativas: 3,
            ultimo: "HTTP 500".into()
        }
    );
    assert_eq!(servidor.pedidos().len(), 3);
}

#[tokio::test]
async fn timeout_devolve_tempo_esgotado_sem_retentar() {
    let servidor = Servidor::iniciar(vec![
        Resposta::ok(sse(&["[DONE]"])).com_atraso(Duration::from_secs(3)),
        Resposta::ok(sse(&["[DONE]"])),
    ])
    .await;
    let m = montar(&servidor.url_base, 2, Duration::from_secs(1));

    let erro = m
        .cliente
        .stream(pergunta("oi"))
        .await
        .err()
        .expect("expira");
    assert_eq!(erro, LlmError::TimedOut { segundos: 1 });
    assert_eq!(
        servidor.pedidos().len(),
        1,
        "timeout não pode ser retentado"
    );
}

#[tokio::test]
async fn cancelamento_no_meio_do_stream_fecha_e_libera_as_vagas() {
    let servidor = Servidor::iniciar(vec![
        Resposta::ok(sse(&[
            r#"{"choices":[{"index":0,"delta":{"content":"Começando"},"finish_reason":null}]}"#,
        ]))
        .deixar_aberta(),
    ])
    .await;
    let m = montar(&servidor.url_base, 0, Duration::from_secs(30));
    let pedido = pergunta("conta uma história longa");
    let cancelamento = pedido.cancelamento.clone();

    let mut fluxo = m.cliente.stream(pedido).await.expect("abre");
    assert_eq!(
        fluxo.next().await.expect("há evento").expect("sem erro"),
        LlmEvent::Texto("Começando".into())
    );
    assert_eq!(
        m.limites.available_globally(),
        3,
        "a chamada segura uma vaga global"
    );

    cancelamento.cancel();
    let proximo = tokio::time::timeout(Duration::from_secs(2), fluxo.next())
        .await
        .expect("cancelamento não pode ficar pendurado");
    assert_eq!(proximo, Some(Err(LlmError::Cancelled)));
    assert!(
        fluxo.next().await.is_none(),
        "depois do cancelamento o stream acaba"
    );
    assert_eq!(
        m.limites.available_globally(),
        4,
        "as vagas voltam depois do cancelamento"
    );
    assert_eq!(
        m.metricas.llm_errors.value(),
        0,
        "cancelamento não conta como erro"
    );
}

#[tokio::test]
async fn resposta_cortada_vira_erro_e_nao_repete() {
    // Sem [DONE] nem finish_reason: a conexão fecha no meio da resposta.
    let servidor = Servidor::iniciar(vec![Resposta::ok(sse(&[
        r#"{"choices":[{"index":0,"delta":{"content":"Oi"},"finish_reason":null}]}"#,
    ]))])
    .await;
    let m = montar(&servidor.url_base, 2, Duration::from_secs(5));

    let fluxo = m.cliente.stream(pergunta("oi")).await.expect("abre");
    let itens = colher(fluxo).await;
    assert_eq!(itens.len(), 2);
    assert_eq!(itens[0], Ok(LlmEvent::Texto("Oi".into())));
    assert!(matches!(itens[1], Err(LlmError::InvalidResponse(_))));
    assert_eq!(
        servidor.pedidos().len(),
        1,
        "depois de texto já enviado não pode repetir"
    );
}

#[tokio::test]
async fn pedido_que_termina_em_turno_do_modelo_sai_quando_a_regra_esta_desligada() {
    let servidor = Servidor::iniciar(vec![Resposta::ok(sse(&[
        r#"{"choices":[{"index":0,"delta":{"content":"Oi"}}]}"#,
        r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
        "[DONE]",
    ]))])
    .await;
    let m = montar_com_regra(&servidor.url_base, 0, Duration::from_secs(5), false);
    let pedido = pedido_com(vec![
        Message::Usuario(vec![Part::Texto("oi".into())]),
        Message::Assistente {
            texto: "olá".into(),
            chamadas: vec![],
        },
    ]);

    let fluxo = m.cliente.stream(pedido).await.expect("pedido aceito");
    let eventos = so_eventos(colher(fluxo).await);
    assert!(
        eventos.contains(&LlmEvent::Texto("Oi".into())),
        "{eventos:?}"
    );
    let enviados = servidor.pedidos();
    assert_eq!(enviados.len(), 1, "o pedido tem de chegar ao servidor");
    let mensagens = enviados[0]["messages"].as_array().expect("lista");
    assert_eq!(mensagens.last().expect("última")["role"], "assistant");
}

#[tokio::test]
async fn pedido_que_termina_em_turno_do_modelo_nao_sai_do_processo() {
    let servidor = Servidor::iniciar(vec![Resposta::ok(sse(&["[DONE]"]))]).await;
    let m = montar(&servidor.url_base, 2, Duration::from_secs(5));
    let pedido = pedido_com(vec![
        Message::Usuario(vec![Part::Texto("oi".into())]),
        Message::Assistente {
            texto: "olá".into(),
            chamadas: vec![],
        },
    ]);

    let erro = m.cliente.stream(pedido).await.err().expect("recusado");
    assert!(matches!(erro, LlmError::InvalidRequest(_)));
    assert!(
        servidor.pedidos().is_empty(),
        "nada pode ser enviado ao gateway"
    );
}
