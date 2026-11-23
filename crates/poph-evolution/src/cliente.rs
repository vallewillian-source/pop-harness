//! Cliente HTTP da EvolutionAPI (HR-26).
//!
//! Cobre o que o POP Atende usa hoje (`core/integrations/evolution_api.py` e
//! `core/integrations/evolution.py`), com as mesmas rotas e os mesmos corpos — para o Harness não
//! inventar um dialeto próprio do canal:
//!
//! | Operação | Rota |
//! |---|---|
//! | criar instância | `POST /instance/create` |
//! | parear (QR) | `GET /instance/connect/{instância}` |
//! | estado | `GET /instance/connectionState/{instância}` |
//! | listar | `GET /instance/fetchInstances` |
//! | desconectar | `DELETE /instance/logout/{instância}` |
//! | apagar | `DELETE /instance/delete/{instância}` |
//! | definir webhook | `POST /webhook/set/{instância}` |
//! | enviar texto | `POST /message/sendText/{instância}` |
//! | enviar mídia | `POST /message/sendMedia/{instância}` |
//! | baixar mídia | `POST /chat/getBase64FromMediaMessage/{instância}` |
//! | presença | `POST /chat/sendPresence/{instância}` |
//!
//! Regras que valem para todas:
//!
//! - **A chave vai só no cabeçalho `apikey`** e nunca aparece em erro, `Debug` ou log.
//! - **Repetição só onde repetir é seguro.** Leitura (estado, QR, lista) e a configuração do
//!   webhook podem repetir uma vez em queda de rede, prazo, 429 e 5xx. Criar, apagar, desconectar,
//!   **enviar** (texto, arquivo, "digitando") e **baixar mídia** nunca repetem: um 5xx depois de a
//!   Evolution já ter entregue ao WhatsApp faria o cliente receber a mesma mensagem duas vezes.
//! - **A mídia respeita o teto antes de decodificar**: o `base64` é medido pelo tamanho do texto,
//!   então um arquivo de 200 MB é recusado sem nunca ocupar memória.
//! - **A resposta tem teto de tamanho**: o corpo é lido em pedaços e a leitura para ao passar do
//!   teto (1 MB, ou o necessário para uma mídia de 16 MB em base64).
//! - **Erro não leva dado do cliente**: o texto do provedor é limpo (sem número, JID, chave e com
//!   tamanho curto) antes de virar mensagem de erro, e o endereço da requisição sai da mensagem de
//!   rede.
//! - **Contadores**: cada requisição HTTP conta em `poph_chamadas_evolution_total`, e cada uma que
//!   falha conta em `poph_erros_evolution_total`. Recusas feitas antes de sair (arquivo grande,
//!   configuração) não são chamadas.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use poph_types::{MetricasBase, Segredo};
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::{Value, json};

use crate::erro::ErroEvolution;
use crate::texto::{LIMITE_TEXTO, para_whatsapp};

/// Espera antes da retentativa (o POP Atende usa 500 ms).
const ESPERA_RETENTATIVA: Duration = Duration::from_millis(500);

/// Teto de mídia do canal (16 MB, `docs/WHATSAPP_MEDIA.md`).
pub const TETO_MIDIA: u64 = 16 * 1024 * 1024;

/// Teto do corpo de resposta comum (JSON de estado, QR, confirmações).
pub const TETO_CORPO_PADRAO: u64 = 1024 * 1024;

/// Tamanho máximo da mídia pedida de volta, em base64, para um arquivo de `teto` bytes: 4/3 do
/// arquivo mais 1 MB de folga para o envelope JSON.
pub fn teto_corpo_midia(teto: u64) -> u64 {
    teto / 3 * 4 + TETO_CORPO_PADRAO
}

/// Tamanho máximo de texto mostrado de uma mensagem de erro do provedor.
const TAMANHO_MENSAGEM_PROVEDOR: usize = 120;

/// Quem pode repetir a chamada.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Repeticao {
    /// Leitura ou configuração idempotente: pode repetir uma vez (se a config permitir).
    Permitida,
    /// Escrita ou envio: uma tentativa só.
    Nunca,
}

/// Configuração do acesso à Evolution.
#[derive(Clone, Debug)]
pub struct ConfigEvolution {
    /// Endereço base, sem barra final (`http://127.0.0.1:8080`).
    pub url_base: String,
    /// Nome da instância no WhatsApp (o mesmo que aparece na Evolution).
    pub instancia: String,
    /// Prazo de cada tentativa.
    pub timeout: Duration,
    /// Repetições extras de leitura e configuração (0 ou 1). Envio e mídia nunca repetem.
    pub retentativas: u32,
    /// Cabeçalho `Origin` opcional (a Evolution atrás de proxy exige em alguns deploys).
    pub origem: Option<String>,
    /// Contadores do canal, quando o servidor os quer expostos em `/metrics`.
    pub metricas: Option<MetricasBase>,
    chave: Segredo,
}

impl ConfigEvolution {
    /// Monta a configuração e valida o que, se estiver errado, quebra toda chamada.
    pub fn novo(
        url_base: impl Into<String>,
        instancia: impl Into<String>,
        chave: Segredo,
    ) -> Result<Self, ErroEvolution> {
        let url_base = url_base.into().trim().trim_end_matches('/').to_owned();
        if !(url_base.starts_with("http://") || url_base.starts_with("https://")) {
            return Err(ErroEvolution::Configuracao(
                "o endereço da Evolution deve começar com http:// ou https://".to_owned(),
            ));
        }
        let instancia = instancia.into().trim().to_owned();
        if instancia.is_empty() {
            return Err(ErroEvolution::Configuracao(
                "a integração do WhatsApp precisa do nome da instância".to_owned(),
            ));
        }
        if chave.expor().trim().is_empty() {
            return Err(ErroEvolution::Configuracao(
                "a integração do WhatsApp precisa da chave da Evolution (`apikey`)".to_owned(),
            ));
        }
        Ok(Self {
            url_base,
            instancia,
            timeout: Duration::from_secs(20),
            retentativas: 1,
            origem: None,
            metricas: None,
            chave,
        })
    }

    /// Troca o prazo por tentativa (teste de prazo e turno curto).
    pub fn com_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Troca o número de repetições de leitura (0 ou 1).
    pub fn com_retentativas(mut self, retentativas: u32) -> Self {
        self.retentativas = retentativas.min(1);
        self
    }

    /// Liga os contadores `poph_chamadas_evolution_total` e `poph_erros_evolution_total`.
    pub fn com_metricas(mut self, metricas: MetricasBase) -> Self {
        self.metricas = Some(metricas);
        self
    }
}

/// Mídia baixada da Evolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MidiaBaixada {
    /// Conteúdo do arquivo.
    pub bytes: Vec<u8>,
    /// Tipo MIME informado pela Evolution.
    pub mime: String,
    /// Nome do arquivo informado pelo WhatsApp (pode vir vazio).
    pub nome: Option<String>,
}

struct Interno {
    http: reqwest::Client,
    config: ConfigEvolution,
}

/// Cliente da Evolution. Clonar é barato (o `reqwest::Client` é compartilhado).
#[derive(Clone)]
pub struct ClienteEvolution(Arc<Interno>);

impl ClienteEvolution {
    /// Cria o cliente HTTP com prazo por tentativa.
    pub fn novo(config: ConfigEvolution) -> Result<Self, ErroEvolution> {
        let http = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|erro| ErroEvolution::Configuracao(format!("cliente HTTP: {erro}")))?;
        Ok(Self(Arc::new(Interno { http, config })))
    }

    /// Nome da instância configurada.
    pub fn instancia(&self) -> &str {
        &self.0.config.instancia
    }

    // ───────────────────────── ciclo de vida da instância ─────────────────────────

    /// Cria a instância (`integration: WHATSAPP-BAILEYS`, com QR).
    pub async fn criar_instancia(&self) -> Result<Value, ErroEvolution> {
        self.pedir(
            reqwest::Method::POST,
            "/instance/create",
            Some(json!({
                "instanceName": self.0.config.instancia,
                "integration": "WHATSAPP-BAILEYS",
                "qrcode": true,
            })),
            Repeticao::Nunca,
            TETO_CORPO_PADRAO,
        )
        .await
    }

    /// Pede o pareamento. Com `numero` (só dígitos, com DDI), a Evolution também devolve o código
    /// de pareamento de 8 caracteres; sem número, ele vem nulo.
    pub async fn conectar(&self, numero: Option<&str>) -> Result<Pareamento, ErroEvolution> {
        let mut caminho = format!("/instance/connect/{}", self.0.config.instancia);
        let digitos: String = numero
            .unwrap_or_default()
            .chars()
            .filter(char::is_ascii_digit)
            .collect();
        if !digitos.is_empty() {
            caminho.push_str(&format!("?number={digitos}"));
        }
        let corpo = self
            .pedir(
                reqwest::Method::GET,
                &caminho,
                None,
                Repeticao::Permitida,
                TETO_CORPO_PADRAO,
            )
            .await?;
        Ok(extrair_pareamento(&corpo))
    }

    /// Estado da conexão: `open`, `connecting` ou `close` (o que a Evolution devolver).
    pub async fn estado_conexao(&self) -> Result<String, ErroEvolution> {
        let caminho = format!("/instance/connectionState/{}", self.0.config.instancia);
        let corpo = self
            .pedir(
                reqwest::Method::GET,
                &caminho,
                None,
                Repeticao::Permitida,
                TETO_CORPO_PADRAO,
            )
            .await?;
        let estado = corpo
            .get("instance")
            .and_then(|instancia| instancia.get("state"))
            .or_else(|| corpo.get("state"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        if estado.is_empty() {
            return Err(ErroEvolution::RespostaInvalida(
                "a resposta do estado da instância não traz `state`".to_owned(),
            ));
        }
        Ok(estado)
    }

    /// Instâncias existentes na Evolution.
    pub async fn listar_instancias(&self) -> Result<Value, ErroEvolution> {
        self.pedir(
            reqwest::Method::GET,
            "/instance/fetchInstances",
            None,
            Repeticao::Permitida,
            TETO_CORPO_PADRAO,
        )
        .await
    }

    /// Desconecta o aparelho sem apagar a instância.
    pub async fn desconectar(&self) -> Result<Value, ErroEvolution> {
        let caminho = format!("/instance/logout/{}", self.0.config.instancia);
        self.pedir(
            reqwest::Method::DELETE,
            &caminho,
            None,
            Repeticao::Nunca,
            TETO_CORPO_PADRAO,
        )
        .await
    }

    /// Apaga a instância.
    pub async fn apagar(&self) -> Result<Value, ErroEvolution> {
        let caminho = format!("/instance/delete/{}", self.0.config.instancia);
        self.pedir(
            reqwest::Method::DELETE,
            &caminho,
            None,
            Repeticao::Nunca,
            TETO_CORPO_PADRAO,
        )
        .await
    }

    /// Liga o webhook da instância para o endereço do Harness.
    ///
    /// `headers` vai junto em toda entrega: é por ali que o segredo do webhook viaja (o POP Atende
    /// usa o cabeçalho `X-Evolution-Webhook-Secret`, conferido do nosso lado).
    pub async fn definir_webhook(
        &self,
        url: &str,
        eventos: &[&str],
        headers: &[(String, String)],
    ) -> Result<Value, ErroEvolution> {
        if url.trim().is_empty() {
            return Err(ErroEvolution::Configuracao(
                "o endereço do webhook está vazio".to_owned(),
            ));
        }
        let eventos: Vec<Value> = eventos
            .iter()
            .map(|evento| Value::String((*evento).to_owned()))
            .collect();
        let cabecalhos: serde_json::Map<String, Value> = headers
            .iter()
            .map(|(nome, valor)| (nome.clone(), Value::String(valor.clone())))
            .collect();
        let caminho = format!("/webhook/set/{}", self.0.config.instancia);
        self.pedir(
            reqwest::Method::POST,
            &caminho,
            Some(json!({
                "webhook": {
                    "enabled": true,
                    "url": url.trim(),
                    "byEvents": false,
                    "base64": false,
                    "headers": Value::Object(cabecalhos),
                    "events": eventos,
                }
            })),
            // Configuração idempotente: repetir a mesma ajusta o mesmo valor.
            Repeticao::Permitida,
            TETO_CORPO_PADRAO,
        )
        .await
    }

    // ───────────────────────── saída ─────────────────────────

    /// Envia texto, aplicando a formatação e o limite do canal.
    ///
    /// O texto já sai daqui no formato que o WhatsApp renderiza ([`para_whatsapp`]) e cortado em
    /// [`LIMITE_TEXTO`] — a Evolution trunca por conta própria e o corte dela é pior (no meio da
    /// frase, sem aviso).
    pub async fn enviar_texto(
        &self,
        numero: &str,
        texto: &str,
    ) -> Result<Option<String>, ErroEvolution> {
        let numero = numero.trim();
        if numero.is_empty() {
            return Err(ErroEvolution::Configuracao(
                "envio sem número de destino".to_owned(),
            ));
        }
        let corpo = para_whatsapp(texto);
        if corpo.trim().is_empty() {
            return Err(ErroEvolution::Configuracao(
                "envio de texto vazio".to_owned(),
            ));
        }
        let caminho = format!("/message/sendText/{}", self.0.config.instancia);
        let resposta = self
            .pedir(
                reqwest::Method::POST,
                &caminho,
                Some(json!({"number": numero, "text": corpo})),
                Repeticao::Nunca,
                TETO_CORPO_PADRAO,
            )
            .await?;
        Ok(id_da_mensagem_enviada(&resposta))
    }

    /// Envia um arquivo (imagem, documento, áudio ou vídeo) por base64.
    ///
    /// O `mediatype` do `sendMedia` sai do MIME: imagem vira `image`, e **todo o resto** vira
    /// `document` — inclusive áudio e vídeo, que desde a MX-48 podem sair do Element. É de propósito:
    /// a rota própria de voz da Evolution (`sendWhatsAppAudio`) exige formato e `ptt` específicos e
    /// não foi provada contra a Evolution de verdade, e um áudio ou vídeo entregue como documento
    /// chega ao cliente de qualquer jeito (o `mimetype` real vai junto). O que se perde é só o
    /// tocador nativo do WhatsApp: o arquivo abre depois de baixar.
    pub async fn enviar_midia(
        &self,
        numero: &str,
        bytes: &[u8],
        mime: &str,
        nome: &str,
        legenda: Option<&str>,
    ) -> Result<Option<String>, ErroEvolution> {
        let numero = numero.trim();
        if numero.is_empty() {
            return Err(ErroEvolution::Configuracao(
                "envio sem número de destino".to_owned(),
            ));
        }
        let tamanho = bytes.len() as u64;
        if tamanho > TETO_MIDIA {
            return Err(ErroEvolution::MidiaGrandeDemais {
                bytes: tamanho,
                teto: TETO_MIDIA,
            });
        }
        if tamanho == 0 {
            return Err(ErroEvolution::Configuracao(
                "envio de arquivo vazio".to_owned(),
            ));
        }
        let mediatipo = if mime.trim().to_ascii_lowercase().starts_with("image/") {
            "image"
        } else {
            "document"
        };
        let mut corpo = json!({
            "number": numero,
            "mediatype": mediatipo,
            "mimetype": mime,
            "fileName": nome,
            "media": base64::engine::general_purpose::STANDARD.encode(bytes),
        });
        if let Some(legenda) = legenda.filter(|legenda| !legenda.trim().is_empty()) {
            corpo["caption"] = Value::String(cortar_legenda(legenda));
        }
        let caminho = format!("/message/sendMedia/{}", self.0.config.instancia);
        let resposta = self
            .pedir(
                reqwest::Method::POST,
                &caminho,
                Some(corpo),
                Repeticao::Nunca,
                TETO_CORPO_PADRAO,
            )
            .await?;
        Ok(id_da_mensagem_enviada(&resposta))
    }

    /// Marca "digitando" ou "pausado" para o cliente.
    pub async fn enviar_presenca(
        &self,
        numero: &str,
        digitando: bool,
    ) -> Result<(), ErroEvolution> {
        let numero = numero.trim();
        if numero.is_empty() {
            return Ok(());
        }
        let caminho = format!("/chat/sendPresence/{}", self.0.config.instancia);
        let presenca = if digitando { "composing" } else { "paused" };
        self.pedir(
            reqwest::Method::POST,
            &caminho,
            Some(json!({"number": numero, "presence": presenca})),
            Repeticao::Nunca,
            TETO_CORPO_PADRAO,
        )
        .await
        .map(|_| ())
    }

    // ───────────────────────── mídia recebida ─────────────────────────

    /// Baixa a mídia de uma mensagem recebida.
    ///
    /// `dados` é o objeto `data` do webhook: só com o id a Evolution procura a mensagem no banco
    /// dela e responde "Message not found" quando não guardou.
    pub async fn baixar_midia(
        &self,
        message_id: &str,
        dados: &Value,
        teto: u64,
    ) -> Result<MidiaBaixada, ErroEvolution> {
        let id_na_chave = dados
            .get("key")
            .and_then(|chave| chave.get("id"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if id_na_chave != message_id {
            return Err(ErroEvolution::RespostaInvalida(
                "o id pedido não é o da mensagem enviada no `data`".to_owned(),
            ));
        }
        let caminho = format!(
            "/chat/getBase64FromMediaMessage/{}",
            self.0.config.instancia
        );
        let resposta = match self
            .pedir(
                reqwest::Method::POST,
                &caminho,
                Some(json!({"message": dados})),
                Repeticao::Nunca,
                teto_corpo_midia(teto),
            )
            .await
        {
            Ok(resposta) => resposta,
            Err(ErroEvolution::Resposta { status, mensagem })
                if eh_midia_expirada(status, &mensagem) =>
            {
                return Err(ErroEvolution::MidiaExpirada(mensagem));
            }
            Err(erro) => return Err(erro),
        };

        let corpo = resposta.get("data").unwrap_or(&resposta);
        let Some(codificado) = corpo.get("base64").and_then(Value::as_str) else {
            return Err(ErroEvolution::RespostaInvalida(
                "a resposta não traz a mídia em `base64`".to_owned(),
            ));
        };
        let sem_espacos: String = codificado.chars().filter(|c| !c.is_whitespace()).collect();
        // O tamanho aproximado é conferido ANTES de decodificar: um arquivo enorme nunca é
        // carregado na memória só para ser recusado depois.
        let estimado = (sem_espacos.len() as u64).saturating_mul(3) / 4;
        if estimado > teto {
            return Err(ErroEvolution::MidiaGrandeDemais {
                bytes: estimado,
                teto,
            });
        }
        let corpo_da_imagem = sem_espacos
            .split_once(',')
            .filter(|(prefixo, _)| prefixo.starts_with("data:"))
            .map(|(_, resto)| resto.to_owned())
            .unwrap_or(sem_espacos);
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(corpo_da_imagem.as_bytes())
            .map_err(|erro| {
                ErroEvolution::RespostaInvalida(format!("base64 da mídia inválido: {erro}"))
            })?;
        if bytes.len() as u64 > teto {
            return Err(ErroEvolution::MidiaGrandeDemais {
                bytes: bytes.len() as u64,
                teto,
            });
        }
        Ok(MidiaBaixada {
            bytes,
            mime: corpo
                .get("mimetype")
                .and_then(Value::as_str)
                .unwrap_or("application/octet-stream")
                .to_owned(),
            nome: corpo
                .get("fileName")
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
    }

    // ───────────────────────── transporte ─────────────────────────

    async fn pedir(
        &self,
        metodo: reqwest::Method,
        caminho: &str,
        corpo: Option<Value>,
        repeticao: Repeticao,
        teto_corpo: u64,
    ) -> Result<Value, ErroEvolution> {
        let url = format!("{}{}", self.0.config.url_base, caminho);
        let repeticoes = match repeticao {
            Repeticao::Permitida => self.0.config.retentativas,
            Repeticao::Nunca => 0,
        };
        let mut ultimo: Option<ErroEvolution> = None;
        for tentativa in 0..=repeticoes {
            if tentativa > 0 {
                tokio::time::sleep(ESPERA_RETENTATIVA).await;
            }
            match self.tentar(&metodo, &url, corpo.clone(), teto_corpo).await {
                Ok(valor) => return Ok(valor),
                Err(erro) => {
                    let repetir = erro.transitorio();
                    ultimo = Some(erro);
                    if !repetir {
                        break;
                    }
                }
            }
        }
        Err(ultimo
            .unwrap_or_else(|| ErroEvolution::Rede("a chamada não chegou a ser feita".to_owned())))
    }

    /// Uma requisição HTTP. Conta em `chamadas` e, se falhar, em `erros`.
    async fn tentar(
        &self,
        metodo: &reqwest::Method,
        url: &str,
        corpo: Option<Value>,
        teto_corpo: u64,
    ) -> Result<Value, ErroEvolution> {
        let metricas = self.0.config.metricas.clone();
        if let Some(m) = &metricas {
            m.chamadas_evolution.incrementar(1);
        }
        let resultado = self.tentar_sem_contar(metodo, url, corpo, teto_corpo).await;
        if let (Err(_), Some(m)) = (&resultado, &metricas) {
            m.erros_evolution.incrementar(1);
        }
        resultado
    }

    async fn tentar_sem_contar(
        &self,
        metodo: &reqwest::Method,
        url: &str,
        corpo: Option<Value>,
        teto_corpo: u64,
    ) -> Result<Value, ErroEvolution> {
        let mut cabecalhos = HeaderMap::new();
        cabecalhos.insert(
            "apikey",
            HeaderValue::from_str(self.0.config.chave.expor().trim()).map_err(|_| {
                ErroEvolution::Configuracao(
                    "a chave da Evolution tem caractere que não cabe em cabeçalho HTTP".to_owned(),
                )
            })?,
        );
        if let Some(origem) = self.0.config.origem.as_deref()
            && let Ok(valor) = HeaderValue::from_str(origem)
        {
            cabecalhos.insert("Origin", valor);
        }
        cabecalhos.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let mut pedido = self.0.http.request(metodo.clone(), url).headers(cabecalhos);
        if let Some(corpo) = corpo {
            pedido = pedido.json(&corpo);
        }
        let resposta = match pedido.send().await {
            Ok(resposta) => resposta,
            Err(erro) => {
                let detalhe = if erro.is_timeout() {
                    "tempo esgotado".to_owned()
                } else if erro.is_connect() {
                    "não consegui conectar".to_owned()
                } else {
                    // Sem o endereço: ele leva o nome da instância.
                    erro.without_url().to_string()
                };
                return Err(ErroEvolution::Rede(detalhe));
            }
        };

        let status = resposta.status();
        let mut resposta = resposta;
        let Some(bruto) = ler_corpo(&mut resposta, teto_corpo).await? else {
            return Err(if status.is_success() {
                ErroEvolution::RespostaGrande { teto: teto_corpo }
            } else {
                ErroEvolution::Resposta {
                    status: status.as_u16(),
                    mensagem: "a resposta de erro passou do tamanho aceito".to_owned(),
                }
            });
        };
        let texto = String::from_utf8_lossy(&bruto).into_owned();
        let corpo: Value = if texto.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&texto).unwrap_or(Value::Null)
        };

        if !status.is_success() {
            return Err(ErroEvolution::Resposta {
                status: status.as_u16(),
                mensagem: mensagem_do_provedor(&corpo, self.0.config.chave.expor()),
            });
        }
        if corpo.is_null() {
            return Ok(json!({}));
        }
        Ok(corpo)
    }
}

/// Lê o corpo em pedaços e para ao passar de `teto` bytes. `None` = passou do teto.
async fn ler_corpo(
    resposta: &mut reqwest::Response,
    teto: u64,
) -> Result<Option<Vec<u8>>, ErroEvolution> {
    if resposta
        .content_length()
        .is_some_and(|tamanho| tamanho > teto)
    {
        return Ok(None);
    }
    let mut bruto = Vec::new();
    while let Some(pedaco) = resposta
        .chunk()
        .await
        .map_err(|erro| ErroEvolution::Rede(erro.without_url().to_string()))?
    {
        if bruto.len() as u64 + pedaco.len() as u64 > teto {
            return Ok(None);
        }
        bruto.extend_from_slice(&pedaco);
    }
    Ok(Some(bruto))
}

/// A mensagem do provedor, limpa para sair num erro ou num log.
///
/// Tira o que identifica o cliente (número, JID com `@`, qualquer trecho com 7 dígitos ou mais),
/// tira a chave se ela vier ecoada e corta em [`TAMANHO_MENSAGEM_PROVEDOR`] caracteres. O corpo
/// inteiro nunca é usado.
fn mensagem_do_provedor(corpo: &Value, chave: &str) -> String {
    let mut bruto: Option<String> = None;
    for campo in ["message", "error", "detail"] {
        if let Some(valor) = corpo.get(campo) {
            if let Some(texto) = valor.as_str()
                && !texto.trim().is_empty()
            {
                bruto = Some(texto.to_owned());
                break;
            }
            if let Some(lista) = valor.as_array() {
                let juntos: Vec<&str> = lista.iter().filter_map(Value::as_str).collect();
                if !juntos.is_empty() {
                    bruto = Some(juntos.join("; "));
                    break;
                }
            }
        }
    }
    let Some(bruto) = bruto else {
        return "sem detalhe".to_owned();
    };
    let limpo = if chave.is_empty() {
        bruto
    } else {
        bruto.replace(chave, "[chave ocultada]")
    };
    let mascarado: Vec<String> = limpo
        .split_whitespace()
        .map(|palavra| {
            let digitos = palavra.chars().filter(char::is_ascii_digit).count();
            if palavra.contains('@') || digitos >= 7 {
                "[identificador ocultado]".to_owned()
            } else {
                palavra.to_owned()
            }
        })
        .collect();
    let junto = mascarado.join(" ");
    if junto.chars().count() <= TAMANHO_MENSAGEM_PROVEDOR {
        junto
    } else {
        junto
            .chars()
            .take(TAMANHO_MENSAGEM_PROVEDOR - 1)
            .collect::<String>()
            + "…"
    }
}

/// A Evolution responde 400/404 com "Message not found" quando não guarda mais a mídia (prazo de 4
/// dias). Só vale para o download de mídia; o vocabulário vem do POP Atende
/// (`core/integrations/evolution.py`, que trata "Message not found" da mesma forma).
fn eh_midia_expirada(status: u16, mensagem: &str) -> bool {
    if status != 400 && status != 404 {
        return false;
    }
    let minusculo = mensagem.to_ascii_lowercase();
    minusculo.contains("message not found")
        || minusculo.contains("media not found")
        || minusculo.contains("media not available")
        || minusculo.contains("expirad")
        || minusculo.contains("expired")
}

/// O que o `connect` devolve: imagem do QR (base64), texto do QR e código de pareamento.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Pareamento {
    /// Imagem do QR em base64 (data URL), quando a Evolution a manda.
    pub qr_base64: Option<String>,
    /// Texto do QR (o que o WhatsApp lê), para quem quiser desenhar o próprio.
    pub qr_texto: Option<String>,
    /// Código de 8 caracteres para parear por número. Só vem quando a chamada levou o número.
    pub codigo_pareamento: Option<String>,
    /// Rodada do QR na Evolution (`count`): sobe a cada QR novo.
    ///
    /// A Evolution v2 **não informa prazo de validade**: o QR gira sozinho a cada ~20 s, e o
    /// `count` é o único sinal de que ele mudou. Quem mostra o QR tem de pedir outro quando o
    /// cliente demorar mais que isso.
    pub contador: Option<u64>,
}

fn extrair_pareamento(corpo: &Value) -> Pareamento {
    let texto = |valor: Option<&Value>| {
        valor
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|valor| !valor.is_empty())
            .map(str::to_owned)
    };
    let contador = corpo
        .get("count")
        .or_else(|| corpo.get("qrcode").and_then(|valor| valor.get("count")))
        .and_then(|valor| {
            valor
                .as_u64()
                .or_else(|| valor.as_str().and_then(|texto| texto.trim().parse().ok()))
        });
    Pareamento {
        qr_base64: texto(corpo.get("qrcode").and_then(|valor| valor.get("base64")))
            .or_else(|| texto(corpo.get("base64"))),
        qr_texto: texto(corpo.get("code"))
            .or_else(|| texto(corpo.get("qrcode").and_then(|valor| valor.get("code")))),
        codigo_pareamento: texto(corpo.get("pairingCode")),
        contador,
    }
}

fn id_da_mensagem_enviada(corpo: &Value) -> Option<String> {
    [
        corpo.get("key").and_then(|chave| chave.get("id")),
        corpo
            .get("message")
            .and_then(|valor| valor.get("key"))
            .and_then(|chave| chave.get("id")),
        corpo.get("id"),
        corpo
            .get("data")
            .and_then(|valor| valor.get("key"))
            .and_then(|chave| chave.get("id")),
    ]
    .into_iter()
    .flatten()
    .filter_map(Value::as_str)
    .map(str::to_owned)
    .find(|valor| !valor.trim().is_empty())
}

fn cortar_legenda(legenda: &str) -> String {
    let convertida = para_whatsapp(legenda);
    let teto = LIMITE_TEXTO / 4;
    if convertida.chars().count() <= teto {
        convertida
    } else {
        convertida.chars().take(teto - 1).collect::<String>() + "…"
    }
}
