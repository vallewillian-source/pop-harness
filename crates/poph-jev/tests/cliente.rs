//! Testes do cliente HTTP do Jev contra um servidor local de verdade.
//!
//! O servidor é um `TcpListener` em porta efêmera que responde o roteiro combinado: nada de rede
//! externa e nada de chave real. O que se prova: o contrato do `/v1/systemone`, a retentativa em
//! 429/5xx, o prazo e — o mais importante — que a chave nunca aparece em erro nem em `Debug`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use poph_jev::{ClienteJev, ConfigJev, ErroJev, Pedido, Resposta, TipoPergunta, TransporteJev};
use poph_types::{BaseMetrics, JevConfig, Registry, Secret};

const CHAVE: &str = "chave-do-jev-nao-pode-vazar";

/// Uma resposta do roteiro.
struct Roteiro {
    codigo: u16,
    corpo: String,
    atraso: Duration,
}

fn ok(corpo: &str) -> Roteiro {
    Roteiro {
        codigo: 200,
        corpo: corpo.to_owned(),
        atraso: Duration::ZERO,
    }
}

fn status(codigo: u16) -> Roteiro {
    Roteiro {
        codigo,
        corpo: "{\"erro\":\"falhou\"}".to_owned(),
        atraso: Duration::ZERO,
    }
}

/// Sobe o servidor com o roteiro e devolve a URL e o registro das requisições recebidas.
fn servidor(roteiro: Vec<Roteiro>) -> (String, Arc<Mutex<Vec<String>>>) {
    let lista = TcpListener::bind("127.0.0.1:0").expect("porta efêmera");
    let porta = lista.local_addr().expect("endereço local").port();
    let recebidas = Arc::new(Mutex::new(Vec::new()));
    let registro = Arc::clone(&recebidas);
    std::thread::spawn(move || {
        for passo in roteiro {
            let Ok((fluxo, _)) = lista.accept() else {
                return;
            };
            let pedido = ler_pedido(&fluxo);
            if let Ok(mut guarda) = registro.lock() {
                guarda.push(pedido);
            }
            if !passo.atraso.is_zero() {
                std::thread::sleep(passo.atraso);
            }
            let _ = responder(&fluxo, passo.codigo, &passo.corpo);
        }
    });
    (format!("http://127.0.0.1:{porta}"), recebidas)
}

fn ler_pedido(fluxo: &TcpStream) -> String {
    let mut leitor = BufReader::new(fluxo);
    let mut cabecalhos = String::new();
    loop {
        let mut linha = String::new();
        if leitor.read_line(&mut linha).unwrap_or(0) == 0 {
            return cabecalhos;
        }
        if linha == "\r\n" || linha == "\n" {
            break;
        }
        cabecalhos.push_str(&linha);
    }
    let tamanho = cabecalhos
        .lines()
        .find_map(|linha| {
            let minuscula = linha.to_ascii_lowercase();
            minuscula
                .strip_prefix("content-length:")
                .map(|valor| valor.trim().parse::<usize>().unwrap_or(0))
        })
        .unwrap_or(0);
    let mut corpo = vec![0_u8; tamanho];
    let _ = leitor.read_exact(&mut corpo);
    format!("{cabecalhos}\r\n{}", String::from_utf8_lossy(&corpo))
}

fn responder(mut fluxo: &TcpStream, codigo: u16, corpo: &str) -> std::io::Result<()> {
    let resposta = format!(
        "HTTP/1.1 {codigo} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{corpo}",
        corpo.len()
    );
    fluxo.write_all(resposta.as_bytes())?;
    fluxo.flush()
}

fn config(url_base: &str, timeout_ms: u64, retentativas: u32) -> ConfigJev {
    let jev = JevConfig {
        enabled: true,
        url_base: url_base.to_owned(),
        model: "jev-latest".to_owned(),
        timeout_seconds: 8,
        retries: retentativas,
        ..JevConfig::default()
    };
    ConfigJev::da_config(&jev, Secret::new(CHAVE.to_owned()))
        .expect("configuração do cliente")
        .com_timeout(Duration::from_millis(timeout_ms))
}

fn metricas() -> BaseMetrics {
    BaseMetrics::register(&Registry::new()).expect("métricas base")
}

fn pedido() -> Pedido {
    let mut criterios = std::collections::BTreeMap::new();
    criterios.insert("continuar_agente".to_owned(), "segue".to_owned());
    let mut perguntas = std::collections::BTreeMap::new();
    perguntas.insert(
        "g1_transferir".to_owned(),
        poph_jev::Pergunta::escolha("julgue", criterios),
    );
    Pedido {
        estado: serde_json::json!({"canal": "cli"}),
        modelo: "jev-latest".to_owned(),
        perguntas,
    }
}

#[tokio::test]
async fn envia_o_contrato_e_le_a_resposta() {
    let (url, recebidas) = servidor(vec![ok(
        r#"{"answers":{"g1_transferir":{"type":"choice","choice":"transferir_agora","probabilities":{"transferir_agora":0.9,"continuar_agente":0.1},"confidence":0.9}},"usage":{"input_tokens":21}}"#,
    )]);
    let cliente = ClienteJev::novo(config(&url, 2_000, 1), metricas()).expect("cliente");

    let lote = cliente.julgar(pedido()).await.expect("resposta");

    let resposta = lote.respostas.get("g1_transferir").expect("resposta do G1");
    assert_eq!(resposta.escolha.as_deref(), Some("transferir_agora"));
    assert_eq!(resposta.confianca, 0.9);
    assert!((resposta.margem() - 0.8).abs() < 1e-9);
    assert_eq!(lote.tokens, 21);

    let pedidos = recebidas.lock().expect("registro").clone();
    assert_eq!(pedidos.len(), 1);
    assert!(
        pedidos[0].starts_with("POST /v1/systemone "),
        "início: {}",
        &pedidos[0][..40.min(pedidos[0].len())]
    );
    let minusculo = pedidos[0].to_ascii_lowercase();
    assert!(minusculo.contains(&format!("authorization: bearer {}", CHAVE.to_lowercase())));
    assert!(minusculo.contains("content-type: application/json"));
    assert!(pedidos[0].contains("\"questions\""));
    assert!(pedidos[0].contains("\"g1_transferir\""));
}

#[tokio::test]
async fn retenta_uma_vez_em_erro_transitorio() {
    let (url, recebidas) = servidor(vec![
        status(429),
        ok(r#"{"answers":{"g1_transferir":{"choice":"continuar_agente","confidence":1.0}}}"#),
    ]);
    let cliente = ClienteJev::novo(config(&url, 2_000, 1), metricas()).expect("cliente");

    let lote = cliente
        .julgar(pedido())
        .await
        .expect("a segunda tentativa passa");

    assert_eq!(
        lote.respostas
            .get("g1_transferir")
            .and_then(|r| r.escolha.clone()),
        Some("continuar_agente".to_owned())
    );
    assert_eq!(recebidas.lock().expect("registro").len(), 2);
}

#[tokio::test]
async fn nao_retenta_em_erro_definitivo_e_nao_vaza_a_chave() {
    let (url, recebidas) = servidor(vec![status(400)]);
    let cliente = ClienteJev::novo(config(&url, 2_000, 1), metricas()).expect("cliente");

    let erro = cliente.julgar(pedido()).await.expect_err("400 não retenta");

    assert!(matches!(erro, ErroJev::Status { codigo: 400, .. }));
    assert!(!erro.to_string().contains(CHAVE));
    assert!(!format!("{erro:?}").contains(CHAVE));
    assert_eq!(recebidas.lock().expect("registro").len(), 1);
}

#[tokio::test]
async fn prazo_por_tentativa_devolve_erro_de_tempo() {
    let (url, _recebidas) = servidor(vec![Roteiro {
        codigo: 200,
        corpo: "{}".to_owned(),
        atraso: Duration::from_secs(3),
    }]);
    let cliente = ClienteJev::novo(config(&url, 200, 0), metricas()).expect("cliente");

    let erro = cliente.julgar(pedido()).await.expect_err("prazo estourado");

    assert!(
        matches!(erro, ErroJev::TempoEsgotado { .. }),
        "erro: {erro:?}"
    );
    assert!(erro.eh_transitorio());
}

#[test]
fn configuracao_aceita_https_recusa_endereco_invalido_e_chave_vazia() {
    // HR-10b: o endereço real da TypeSafe é https e passa a ser aceito.
    let https = JevConfig {
        enabled: true,
        url_base: "https://api.typesafe.ai/".to_owned(),
        ..JevConfig::default()
    };
    let config = ConfigJev::da_config(&https, Secret::new("x".to_owned())).expect("https aceito");
    assert_eq!(config.url_base, "https://api.typesafe.ai");
    assert_eq!(config.endpoint(), "https://api.typesafe.ai/v1/systemone");

    let sem_esquema = JevConfig {
        enabled: true,
        url_base: "api.typesafe.ai".to_owned(),
        ..JevConfig::default()
    };
    let erro = ConfigJev::da_config(&sem_esquema, Secret::new("x".to_owned()))
        .expect_err("endereço sem esquema");
    assert!(erro.to_string().contains("http:// ou https://"));

    let http = JevConfig {
        enabled: true,
        url_base: "http://127.0.0.1:9".to_owned(),
        ..JevConfig::default()
    };
    let sem_chave =
        ConfigJev::da_config(&http, Secret::new("   ".to_owned())).expect_err("recusa chave vazia");
    assert!(matches!(sem_chave, ErroJev::SemChave));
    assert!(!format!("{http:?}").contains(CHAVE));
}

#[test]
fn lote_le_o_formato_de_resposta_da_api() {
    // `choice` com probabilidades, `score` e `noul`: os três formatos que a API devolve.
    let corpo = serde_json::json!({
        "answers": {
            "a": {"type": "choice", "choice": "sim", "probabilities": {"sim": 0.8, "nao": 0.2}, "confidence": 0.8},
            "b": {"type": "score", "score": 0.42, "confidence": 0.5},
            "c": {"type": "noul", "noul": 0.11}
        },
        "usage": {"input_tokens": 10, "output_tokens": 5}
    });
    let lote = poph_jev::LoteResposta::de_json(&corpo).expect("lote");
    assert_eq!(lote.respostas.len(), 3);
    assert_eq!(lote.tokens, 15);
    // O `usage` detalhado é lido separado, sem estimar nada.
    assert_eq!(lote.usage.entrada, Some(10));
    assert_eq!(lote.usage.saida, Some(5));
    assert_eq!(
        lote.usage.total,
        Some(15),
        "sem total_tokens, soma entrada e saída"
    );
    let nota: &Resposta = &lote.respostas["b"];
    assert_eq!(nota.score, Some(0.42));
    assert!(nota.escolha.is_none());

    let pergunta = poph_jev::Pergunta::escolha("julgue", Default::default());
    assert_eq!(pergunta.tipo, TipoPergunta::Choice);
}

/// `usage` em outras formas: nomes alternativos do provedor, `total_tokens` explícito e ausência.
#[test]
fn le_usage_em_variantes_e_sem_usage() {
    let alternativo = serde_json::json!({
        "answers": {},
        "usage": {"prompt_tokens": 7, "completion_tokens": 3, "total_tokens": 11}
    });
    let lote = poph_jev::LoteResposta::de_json(&alternativo).expect("lote");
    assert_eq!(lote.usage.entrada, Some(7));
    assert_eq!(lote.usage.saida, Some(3));
    assert_eq!(
        lote.usage.total,
        Some(11),
        "total_tokens informado manda sobre a soma"
    );
    assert_eq!(lote.tokens, 11);
    assert!(lote.usage.informado());

    let sem_usage = serde_json::json!({"answers": {}});
    let lote = poph_jev::LoteResposta::de_json(&sem_usage).expect("lote");
    assert!(!lote.usage.informado());
    assert_eq!(lote.usage.como_texto(), "não informado");
    assert_eq!(lote.tokens, 0);

    let so_entrada = serde_json::json!({"answers": {}, "usage": {"input_tokens": 4}});
    let lote = poph_jev::LoteResposta::de_json(&so_entrada).expect("lote");
    assert_eq!(lote.usage.entrada, Some(4));
    assert_eq!(lote.usage.saida, None);
    assert_eq!(
        lote.usage.total, None,
        "sem um dos lados, não inventa total"
    );
    assert_eq!(
        lote.usage.como_texto(),
        "entrada 4 · saída não informado · total não informado"
    );
}
