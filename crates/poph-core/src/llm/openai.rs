//! Cliente OpenAI-compatível (o `antigravity2api` local é um deles).
//!
//! Regras deste cliente:
//! - Retentativa só em 429, 5xx ou falha de conexão **antes** de a resposta começar. Depois do
//!   primeiro byte da resposta, falha vira erro definitivo: não repete chamada de ferramenta nem
//!   texto que já saiu para quem chamou.
//! - Timeout (`timeout_segundos`) vale por tentativa, do envio até o fim do corpo. Timeout não é
//!   retentado.
//! - O stream entrega a chamada de ferramenta só quando os argumentos chegaram inteiros.
//! - Vagas por org e globais (`LimitadorLlm`) são seguradas até o stream terminar ou ser solto.
//! - A chave vem de `Segredo` e só é exposta no cabeçalho `Authorization`. Mensagens de erro
//!   trocam a chave por `[oculto]` se o provedor ecoar o valor.
//! - `http://` e `https://` são aceitos (HR-10b). O TLS é `rustls` com as raízes do sistema; falha
//!   de TLS (certificado não confiável, nome errado, handshake) é erro definitivo e não é retentada —
//!   queda de rede e prazo continuam transitórios.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use futures::stream;
use poph_types::{LlmConfig, MetricasBase, Segredo};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::{Value, json};

use super::base64;
use super::erro::ErroLlm;
use super::limites::{LimitadorLlm, PermissaoLlm};
use super::provedor::{FluxoLlm, LlmProvider, validar_pedido};
use super::sse::LeitorSse;
use super::tipos::{
    Cancelamento, ChamadaFerramenta, EventoLlm, Ferramenta, Mensagem, MotivoFim, Parte, Pedido, Uso,
};
use super::tls;

pub const PROVEDOR_OPENAI_COMPATIVEL: &str = "openai_compativel";
pub const PROVEDOR_ANTIGRAVITY: &str = "antigravity2api";

/// Teto de espera entre retentativas.
const ESPERA_MAXIMA: Duration = Duration::from_secs(30);

/// Parâmetros do cliente, já validados.
#[derive(Clone, Debug)]
pub struct OpenAiCompatibleConfig {
    /// Endereço base, terminado em `/v1` (ex.: `http://127.0.0.1:8045/v1`).
    pub url_base: String,
    /// Chave de API. Nunca aparece em log, erro ou `Debug`.
    pub chave: Segredo,
    /// Modelo padrão (o pedido pode trocar).
    pub modelo: String,
    /// Tempo máximo de cada tentativa.
    pub timeout: Duration,
    /// Novas tentativas, só em erro transitório.
    pub retentativas: u32,
    /// Espera antes da primeira retentativa; dobra a cada tentativa.
    pub espera_base: Duration,
    /// Temperatura padrão.
    pub temperatura: f32,
    /// Teto de tokens de saída.
    pub max_tokens_saida: u32,
    /// Recusa pedido que termina em turno do modelo (o `antigravity2api` devolve HTTP 400 nesse caso).
    pub recusa_turno_modelo: bool,
}

impl OpenAiCompatibleConfig {
    /// Monta a configuração a partir do bloco `[llm]` e da chave lida do ambiente.
    pub fn da_config(llm: &LlmConfig, chave: Segredo) -> Result<Self, ErroLlm> {
        if !poph_types::provedor_llm_aceito(&llm.provedor) {
            return Err(ErroLlm::Configuracao(format!(
                "provedor {} não suportado; use {PROVEDOR_OPENAI_COMPATIVEL} ou {PROVEDOR_ANTIGRAVITY}",
                llm.provedor
            )));
        }
        if chave.expor().trim().is_empty() {
            return Err(ErroLlm::Configuracao(
                "chave de API do provedor ausente".to_owned(),
            ));
        }
        let url_base = llm.url_base.trim_end_matches('/').to_owned();
        // http (gateway local) e https (provedor com TLS) são aceitos; o TLS usa rustls com as raízes
        // do sistema (HR-10b).
        if !(url_base.starts_with("http://") || url_base.starts_with("https://")) {
            return Err(ErroLlm::Configuracao(
                "o endereço do provedor deve começar com http:// ou https://".to_owned(),
            ));
        }
        Ok(Self {
            url_base,
            chave,
            modelo: llm.modelo.clone(),
            timeout: Duration::from_secs(u64::from(llm.timeout_segundos)),
            retentativas: llm.retentativas,
            espera_base: Duration::from_secs(1),
            temperatura: llm.temperatura,
            max_tokens_saida: llm.max_tokens_saida,
            recusa_turno_modelo: llm.recusa_turno_modelo,
        })
    }
}

struct Interno {
    http: reqwest::Client,
    endpoint: String,
    config: OpenAiCompatibleConfig,
    limites: LimitadorLlm,
    metricas: MetricasBase,
}

/// Cliente OpenAI-compatível. Clonar é barato e todos os clones compartilham limites e métricas.
#[derive(Clone)]
pub struct OpenAiCompatibleClient(Arc<Interno>);

impl OpenAiCompatibleClient {
    /// Cria o cliente. `limites` e `metricas` são compartilhados com o resto da instância.
    pub fn new(
        config: OpenAiCompatibleConfig,
        limites: LimitadorLlm,
        metricas: MetricasBase,
    ) -> Result<Self, ErroLlm> {
        let http = construir_http(None)?;
        let endpoint = format!("{}/chat/completions", config.url_base);
        Ok(Self(Arc::new(Interno {
            http,
            endpoint,
            config,
            limites,
            metricas,
        })))
    }

    /// **Só para teste:** devolve um cliente igual a este que também confia numa raiz extra (DER).
    ///
    /// Serve para o servidor TLS local dos testes usar um certificado próprio. Não desliga a
    /// validação nem existe para produção: é uma raiz a mais, e o certificado continua sendo
    /// conferido (nome, validade, cadeia).
    #[doc(hidden)]
    pub fn com_raiz_de_teste(self, raiz_der: &[u8]) -> Result<Self, ErroLlm> {
        let http = construir_http(Some(raiz_der))?;
        Ok(Self(Arc::new(Interno {
            http,
            endpoint: self.0.endpoint.clone(),
            config: self.0.config.clone(),
            limites: self.0.limites.clone(),
            metricas: self.0.metricas.clone(),
        })))
    }
}

/// Monta o cliente HTTP. Com `raiz_extra`, acrescenta uma raiz de confiança (só teste).
fn construir_http(raiz_extra: Option<&[u8]>) -> Result<reqwest::Client, ErroLlm> {
    let mut construtor = reqwest::Client::builder();
    if let Some(der) = raiz_extra {
        let certificado = reqwest::Certificate::from_der(der).map_err(|erro| {
            ErroLlm::Configuracao(format!(
                "raiz de confiança inválida: {}",
                erro.without_url()
            ))
        })?;
        construtor = construtor.add_root_certificate(certificado);
    }
    construtor
        .build()
        .map_err(|erro| ErroLlm::Configuracao(format!("cliente HTTP: {}", erro.without_url())))
}

impl LlmProvider for OpenAiCompatibleClient {
    fn stream<'a>(&'a self, pedido: Pedido) -> BoxFuture<'a, Result<FluxoLlm, ErroLlm>> {
        Box::pin(abrir(Arc::clone(&self.0), pedido))
    }
}

async fn abrir(interno: Arc<Interno>, pedido: Pedido) -> Result<FluxoLlm, ErroLlm> {
    validar_pedido(&pedido, interno.config.recusa_turno_modelo)
        .map_err(|erro| interno.registrar_falha(erro))?;
    interno.metricas.chamadas_llm.incrementar(1);

    let cancelamento = pedido.cancelamento.clone();
    let permissao: PermissaoLlm = tokio::select! {
        biased;
        _ = cancelamento.aguardar() => return Err(ErroLlm::Cancelada),
        resultado = interno.limites.adquirir(pedido.org) => {
            resultado.map_err(|erro| interno.registrar_falha(erro))?
        }
    };

    let corpo = montar_corpo(&pedido, &interno.config);
    let modelo = pedido
        .modelo
        .clone()
        .unwrap_or_else(|| interno.config.modelo.clone());
    let inicio = Instant::now();
    let resposta = enviar(&interno, &corpo, &cancelamento)
        .await
        .map_err(|erro| interno.registrar_falha(erro))?;

    tracing::debug!(modelo = %modelo, "resposta do LLM começou a chegar");
    let estado = Estado {
        interno: Arc::clone(&interno),
        resposta,
        leitor: LeitorSse::new(),
        parciais: BTreeMap::new(),
        pendentes: VecDeque::new(),
        motivo: None,
        uso: None,
        encerrado: false,
        permissao: Some(permissao),
        cancelamento,
        inicio,
        modelo,
        org: pedido.org,
    };
    Ok(Box::pin(stream::unfold(estado, avancar)))
}

impl Interno {
    /// Conta a falha nas métricas e devolve o mesmo erro. Cancelamento não conta como falha.
    fn registrar_falha(&self, erro: ErroLlm) -> ErroLlm {
        if erro != ErroLlm::Cancelada {
            self.metricas.erros_llm.incrementar(1);
            tracing::warn!(erro = %erro, "chamada ao LLM falhou");
        }
        erro
    }
}

/// Envia o pedido, com retentativas só em falha transitória antes da resposta começar.
async fn enviar(
    interno: &Interno,
    corpo: &Value,
    cancelamento: &Cancelamento,
) -> Result<reqwest::Response, ErroLlm> {
    let total = interno.config.retentativas + 1;
    let corpo_texto = corpo.to_string();
    let mut ultimo = String::from("sem resposta");

    for tentativa in 1..=total {
        let envio = interno
            .http
            .post(&interno.endpoint)
            .timeout(interno.config.timeout)
            .header(
                AUTHORIZATION,
                format!("Bearer {}", interno.config.chave.expor()),
            )
            .header(CONTENT_TYPE, "application/json")
            .body(corpo_texto.clone())
            .send();

        let resultado = tokio::select! {
            biased;
            _ = cancelamento.aguardar() => return Err(ErroLlm::Cancelada),
            resultado = envio => resultado,
        };

        match resultado {
            Ok(resposta) if resposta.status().is_success() => return Ok(resposta),
            Ok(resposta) => {
                let status = resposta.status().as_u16();
                let texto = resposta.text().await.unwrap_or_default();
                if status == 429 || (500..=599).contains(&status) {
                    ultimo = format!("HTTP {status}");
                } else {
                    return Err(ErroLlm::Recusado {
                        status,
                        mensagem: resumir_erro(&texto, interno.config.chave.expor()),
                    });
                }
            }
            Err(erro) if erro.is_timeout() => {
                return Err(ErroLlm::TempoEsgotado {
                    segundos: interno.config.timeout.as_secs(),
                });
            }
            Err(erro) => {
                // Falha de TLS é definitiva: repetir não conserta certificado nem nome errado.
                if let Some(detalhe) = tls::detalhe_de_falha_tls(&erro) {
                    return Err(ErroLlm::Tls(detalhe));
                }
                ultimo = if erro.is_connect() {
                    "conexão recusada".to_owned()
                } else {
                    "falha de rede".to_owned()
                };
            }
        }

        if tentativa < total {
            let espera = espera_da_tentativa(interno.config.espera_base, tentativa);
            tracing::info!(tentativa, motivo = %ultimo, "nova tentativa no LLM");
            let seguir = tokio::select! {
                biased;
                _ = cancelamento.aguardar() => false,
                _ = tokio::time::sleep(espera) => true,
            };
            if !seguir {
                return Err(ErroLlm::Cancelada);
            }
        }
    }
    Err(ErroLlm::Indisponivel {
        tentativas: total,
        ultimo,
    })
}

fn espera_da_tentativa(base: Duration, tentativa: u32) -> Duration {
    let fator = 1u32 << (tentativa - 1).min(10);
    base.saturating_mul(fator).min(ESPERA_MAXIMA)
}

/// Tira do texto de erro do provedor a mensagem útil, sem a chave, e limita o tamanho.
fn resumir_erro(texto: &str, chave: &str) -> String {
    let mensagem = serde_json::from_str::<Value>(texto)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
        .unwrap_or_else(|| texto.to_owned());
    let mut limpo: String = mensagem.chars().take(300).collect();
    if !chave.is_empty() {
        limpo = limpo.replace(chave, "[oculto]");
    }
    limpo
}

/// Monta o corpo do pedido no formato OpenAI (que o gateway aceita).
pub fn montar_corpo(pedido: &Pedido, config: &OpenAiCompatibleConfig) -> Value {
    let modelo = pedido.modelo.as_deref().unwrap_or(&config.modelo);
    let mensagens: Vec<Value> = pedido.mensagens.iter().map(mensagem_json).collect();
    // Temperatura e teto de tokens podem vir do agente (configuração por agente); o bloco `[llm]`
    // é o padrão quando o pedido não traz nada.
    let temperatura = pedido.temperatura.unwrap_or(config.temperatura);
    let max_tokens = pedido.max_tokens.unwrap_or(config.max_tokens_saida);
    let mut corpo = json!({
        "model": modelo,
        "stream": true,
        "temperature": temperatura,
        "max_tokens": max_tokens,
        "messages": mensagens,
    });
    if !pedido.ferramentas.is_empty() {
        corpo["tools"] = Value::Array(pedido.ferramentas.iter().map(ferramenta_json).collect());
    }
    corpo
}

fn mensagem_json(mensagem: &Mensagem) -> Value {
    match mensagem {
        Mensagem::Sistema(texto) => json!({"role": "system", "content": texto}),
        Mensagem::Usuario(partes) => {
            if let [Parte::Texto(texto)] = partes.as_slice() {
                return json!({"role": "user", "content": texto});
            }
            let conteudo: Vec<Value> = partes.iter().map(parte_json).collect();
            json!({"role": "user", "content": conteudo})
        }
        Mensagem::Assistente { texto, chamadas } => {
            let mut obj = json!({"role": "assistant"});
            obj["content"] = if texto.is_empty() && !chamadas.is_empty() {
                Value::Null
            } else {
                Value::String(texto.clone())
            };
            if !chamadas.is_empty() {
                obj["tool_calls"] = Value::Array(
                    chamadas
                        .iter()
                        .map(|c| {
                            json!({
                                "id": c.id,
                                "type": "function",
                                "function": {"name": c.nome, "arguments": c.argumentos.to_string()},
                            })
                        })
                        .collect(),
                );
            }
            obj
        }
        Mensagem::Resultado { id, conteudo } => {
            json!({"role": "tool", "tool_call_id": id, "content": conteudo})
        }
    }
}

fn parte_json(parte: &Parte) -> Value {
    match parte {
        Parte::Texto(texto) => json!({"type": "text", "text": texto}),
        Parte::Imagem { mime, bytes } => json!({
            "type": "image_url",
            "image_url": {"url": format!("data:{mime};base64,{}", base64::codificar(bytes))},
        }),
    }
}

fn ferramenta_json(ferramenta: &Ferramenta) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": ferramenta.nome,
            "description": ferramenta.descricao,
            "parameters": ferramenta.parametros,
        },
    })
}

/// Chamada de ferramenta em montagem: os pedaços chegam em deltas, indexados.
#[derive(Default)]
struct Parcial {
    id: Option<String>,
    nome: Option<String>,
    argumentos: String,
}

/// Estado de leitura de um stream. Guarda as vagas até o fim.
struct Estado {
    interno: Arc<Interno>,
    resposta: reqwest::Response,
    leitor: LeitorSse,
    parciais: BTreeMap<u64, Parcial>,
    pendentes: VecDeque<Result<EventoLlm, ErroLlm>>,
    motivo: Option<MotivoFim>,
    uso: Option<Uso>,
    encerrado: bool,
    permissao: Option<PermissaoLlm>,
    cancelamento: Cancelamento,
    inicio: Instant,
    modelo: String,
    org: poph_types::OrgId,
}

async fn avancar(mut e: Estado) -> Option<(Result<EventoLlm, ErroLlm>, Estado)> {
    loop {
        if let Some(item) = e.pendentes.pop_front() {
            if item.is_err() {
                e.encerrado = true;
                e.permissao = None;
            }
            return Some((item, e));
        }
        if e.encerrado {
            return None;
        }

        let pedaco = tokio::select! {
            biased;
            _ = e.cancelamento.aguardar() => Err(ErroLlm::Cancelada),
            pedaco = e.resposta.chunk() => pedaco.map_err(|erro| {
                if erro.is_timeout() {
                    ErroLlm::TempoEsgotado { segundos: e.interno.config.timeout.as_secs() }
                } else {
                    ErroLlm::ConexaoInterrompida("a conexão caiu no meio da resposta".to_owned())
                }
            }),
        };

        match pedaco {
            Err(erro) => {
                e.encerrado = true;
                e.permissao = None;
                let erro = e.interno.registrar_falha(erro);
                e.pendentes.push_back(Err(erro));
            }
            Ok(Some(bytes)) => {
                for payload in e.leitor.empurrar(&bytes) {
                    if e.encerrado {
                        break;
                    }
                    processar_payload(&mut e, &payload);
                }
            }
            Ok(None) => {
                for payload in e.leitor.encerrar() {
                    if e.encerrado {
                        break;
                    }
                    processar_payload(&mut e, &payload);
                }
                if !e.encerrado {
                    if e.motivo.is_none() {
                        let erro = e.interno.registrar_falha(ErroLlm::RespostaInvalida(
                            "a resposta terminou antes do fim (conexão fechada sem motivo de fim)"
                                .to_owned(),
                        ));
                        e.pendentes.push_back(Err(erro));
                        e.encerrado = true;
                        e.permissao = None;
                    } else {
                        finalizar(&mut e);
                    }
                }
            }
        }
    }
}

/// Trata um payload `data:` do stream: texto, ferramenta, uso, motivo de fim ou erro.
fn processar_payload(e: &mut Estado, payload: &str) {
    if payload == "[DONE]" {
        finalizar(e);
        return;
    }
    let Ok(valor) = serde_json::from_str::<Value>(payload) else {
        tracing::debug!("evento do stream do LLM sem JSON válido, ignorado");
        return;
    };
    if !valor["error"].is_null() {
        let mensagem = valor["error"]["message"]
            .as_str()
            .map(|m| resumir_erro(m, e.interno.config.chave.expor()))
            .unwrap_or_else(|| "erro sem detalhe".to_owned());
        let erro = e.interno.registrar_falha(ErroLlm::ErroNoStream(mensagem));
        e.pendentes.push_back(Err(erro));
        e.encerrado = true;
        e.permissao = None;
        return;
    }
    if let Some(uso) = valor.get("usage").filter(|u| !u.is_null()) {
        e.uso = Some(Uso {
            entrada: uso["prompt_tokens"].as_u64().unwrap_or(0),
            saida: uso["completion_tokens"].as_u64().unwrap_or(0),
        });
    }
    let escolha = &valor["choices"][0];
    let delta = &escolha["delta"];
    if let Some(texto) = delta["content"].as_str().filter(|t| !t.is_empty()) {
        e.pendentes
            .push_back(Ok(EventoLlm::Texto(texto.to_owned())));
    }
    if let Some(chamadas) = delta["tool_calls"].as_array() {
        for chamada in chamadas {
            let indice = chamada["index"].as_u64().unwrap_or(0);
            let parcial = e.parciais.entry(indice).or_default();
            if let Some(id) = chamada["id"].as_str().filter(|s| !s.is_empty()) {
                parcial.id = Some(id.to_owned());
            }
            if let Some(nome) = chamada["function"]["name"]
                .as_str()
                .filter(|s| !s.is_empty())
            {
                parcial.nome = Some(nome.to_owned());
            }
            if let Some(pedaco) = chamada["function"]["arguments"].as_str() {
                parcial.argumentos.push_str(pedaco);
            }
        }
    }
    if let Some(motivo) = escolha["finish_reason"].as_str() {
        e.motivo = Some(match motivo {
            "stop" => MotivoFim::Concluido,
            "tool_calls" | "function_call" => MotivoFim::ChamadasFerramenta,
            "length" => MotivoFim::LimiteDeTokens,
            outro => MotivoFim::Outro(outro.to_owned()),
        });
    }
}

/// Fecha a resposta: emite as ferramentas completas, o uso e o fim, e solta as vagas.
fn finalizar(e: &mut Estado) {
    if e.encerrado {
        return;
    }
    e.encerrado = true;
    e.permissao = None;

    let parciais = std::mem::take(&mut e.parciais);
    let mut total_ferramentas = 0usize;
    for (indice, parcial) in parciais {
        let Some(nome) = parcial.nome else {
            let erro = e.interno.registrar_falha(ErroLlm::RespostaInvalida(
                "chamada de ferramenta sem nome".to_owned(),
            ));
            e.pendentes.push_back(Err(erro));
            return;
        };
        let argumentos = if parcial.argumentos.trim().is_empty() {
            json!({})
        } else {
            match serde_json::from_str::<Value>(&parcial.argumentos) {
                Ok(valor) => valor,
                Err(_) => {
                    let erro = e.interno.registrar_falha(ErroLlm::RespostaInvalida(format!(
                        "argumentos da ferramenta {nome} não são JSON válido"
                    )));
                    e.pendentes.push_back(Err(erro));
                    return;
                }
            }
        };
        let id = parcial.id.unwrap_or_else(|| format!("chamada_{indice}"));
        total_ferramentas += 1;
        e.pendentes
            .push_back(Ok(EventoLlm::Ferramenta(ChamadaFerramenta {
                id,
                nome,
                argumentos,
            })));
    }

    if let Some(uso) = e.uso {
        e.interno.metricas.tokens_total.incrementar(uso.total());
        e.pendentes.push_back(Ok(EventoLlm::Uso(uso)));
    }
    let motivo = match e.motivo.take() {
        Some(motivo) => motivo,
        None if total_ferramentas > 0 => MotivoFim::ChamadasFerramenta,
        None => MotivoFim::Concluido,
    };
    e.pendentes.push_back(Ok(EventoLlm::Fim(motivo)));

    tracing::info!(
        org = %e.org,
        modelo = %e.modelo,
        ferramentas = total_ferramentas,
        duracao_ms = e.inicio.elapsed().as_millis() as u64,
        "resposta do LLM concluída"
    );
}

#[cfg(test)]
mod testes {
    use poph_types::OrgId;

    use super::*;

    fn config(url: &str, chave: &str) -> Result<OpenAiCompatibleConfig, ErroLlm> {
        let llm = LlmConfig {
            provedor: PROVEDOR_OPENAI_COMPATIVEL.to_owned(),
            url_base: url.to_owned(),
            chave_env: "ANTIGRAVITY_API_KEY".to_owned(),
            recusa_turno_modelo: true,
            modelo: "gemini-3.8-flash-tiered".to_owned(),
            timeout_segundos: 10,
            retentativas: 2,
            simultaneos: 4,
            temperatura: 0.0,
            max_tokens_saida: 512,
        };
        OpenAiCompatibleConfig::da_config(&llm, Segredo::new(chave.to_owned()))
    }

    #[test]
    fn aceita_https_recusa_endereco_invalido_e_chave_vazia() {
        // HR-10b: https é aceito (TLS via rustls); só o que não é http(s) é recusado.
        let https = config("https://exemplo.test/v1", "k").expect("https aceito");
        assert_eq!(https.url_base, "https://exemplo.test/v1");
        let sem_esquema = config("exemplo.test/v1", "k").unwrap_err();
        assert!(
            matches!(sem_esquema, ErroLlm::Configuracao(m) if m.contains("http:// ou https://"))
        );
        assert!(config("http://127.0.0.1:8045/v1/", "").is_err());
        let llm = LlmConfig {
            provedor: "outro".to_owned(),
            url_base: "http://127.0.0.1:8045/v1".to_owned(),
            chave_env: "ANTIGRAVITY_API_KEY".to_owned(),
            recusa_turno_modelo: true,
            modelo: "m".to_owned(),
            timeout_segundos: 10,
            retentativas: 0,
            simultaneos: 1,
            temperatura: 0.0,
            max_tokens_saida: 1,
        };
        assert!(OpenAiCompatibleConfig::da_config(&llm, Segredo::new("k".into())).is_err());
    }

    #[test]
    fn monta_corpo_com_imagem_ferramenta_e_modelo_do_pedido() {
        let cfg = config("http://127.0.0.1:8045/v1", "segredo-de-teste").expect("config");
        let mut pedido = Pedido::new(
            OrgId::generate(),
            vec![
                Mensagem::Sistema("instruções".into()),
                Mensagem::Usuario(vec![
                    Parte::Texto("o que tem na foto?".into()),
                    Parte::Imagem {
                        mime: "image/png".into(),
                        bytes: b"foo".to_vec(),
                    },
                ]),
            ],
        );
        pedido.modelo = Some("outro-modelo".into());
        pedido.ferramentas = vec![Ferramenta {
            nome: "buscar".into(),
            descricao: "busca".into(),
            parametros: json!({"type": "object"}),
        }];
        let corpo = montar_corpo(&pedido, &cfg);
        assert_eq!(corpo["model"], "outro-modelo");
        assert_eq!(corpo["stream"], true);
        assert_eq!(corpo["messages"][0]["role"], "system");
        let partes = &corpo["messages"][1]["content"];
        assert_eq!(partes[1]["image_url"]["url"], "data:image/png;base64,Zm9v");
        assert_eq!(corpo["tools"][0]["function"]["name"], "buscar");
        assert!(!corpo.to_string().contains("segredo-de-teste"));
    }

    #[test]
    fn espera_dobra_e_tem_teto() {
        let base = Duration::from_secs(1);
        assert_eq!(espera_da_tentativa(base, 1), Duration::from_secs(1));
        assert_eq!(espera_da_tentativa(base, 2), Duration::from_secs(2));
        assert_eq!(espera_da_tentativa(base, 3), Duration::from_secs(4));
        assert_eq!(espera_da_tentativa(base, 30), ESPERA_MAXIMA);
    }

    #[test]
    fn erro_do_provedor_nao_ecoa_a_chave() {
        let corpo = r#"{"error":{"message":"chave inválida: abc-123-segredo"}}"#;
        let texto = resumir_erro(corpo, "abc-123-segredo");
        assert!(!texto.contains("abc-123-segredo"));
        assert!(texto.contains("[oculto]"));
    }
}
