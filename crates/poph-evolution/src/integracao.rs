//! O canal WhatsApp como [`Integration`] nativa (HR-26).
//!
//! Junta as três peças — cliente da Evolution ([`crate::cliente`]), leitura do webhook
//! ([`crate::webhook`]) e formatação/limite de texto ([`crate::texto`]) — no mesmo contrato que o
//! plugin em processo externo usa (`poph_types::Integration`). Quem entrega ao canal é a `outbox`
//! da HR-12, por `EnvioPeloCanal`; este módulo não conhece fila, turno nem banco.
//!
//! Escolhas de projeto:
//!
//! - **A instância mora na configuração da integração**, não no código: cada org tem a sua na
//!   Evolution, e o Console (HR-44) é quem preenche.
//! - **O segredo do webhook é obrigatório** para receber: sem ele, o Harness recusaria só na hora
//!   da entrega; é melhor falhar cedo, na configuração.
//! - **Arquivo enviado sai da biblioteca do agente** (D13): o comando traz `blob_ref` e a leitura é
//!   feita por [`FonteDeMidia`], que recusa caminho que escape da raiz dos blobs.
//! - **"Digitando" é opcional e não derruba o turno**: se a Evolution não aceitar a presença, o
//!   erro é registrado e a mensagem segue (presença não é conteúdo).

use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use poph_types::{
    Capacidades, ComandoSaida, ErroIntegracao, EsquemaJson, EstadoIntegracao, FuturoComando,
    FuturoSaude, InboundEvent, Integration, IntegrationId, LimitesCanal, Manifesto,
    PROTOCOLO_VERSAO, Presenca, Saude, Segredo, SendFile, SendText, SetPresence,
};

use crate::cliente::{ClienteEvolution, TETO_MIDIA};
use crate::erro::ErroEvolution;
use crate::texto::LIMITE_TEXTO;
use crate::webhook::{
    CABECALHO_SEGREDO, Decisao, ErroWebhook, Politica, conferir_segredo, interpretar,
};

/// Configuração do canal WhatsApp.
#[derive(Clone, Debug)]
pub struct ConfigCanal {
    /// Qual integração do banco este canal representa (a `integrations` da API).
    pub integracao: IntegrationId,
    /// Endereço em que a Evolution entrega o webhook (`https://harness.exemplo/v1/webhooks/whatsapp`).
    /// Vazio = não mexe no webhook (a instância já foi configurada à mão).
    pub webhook_url: Option<String>,
    /// Segredo que a Evolution devolve no cabeçalho [`CABECALHO_SEGREDO`]. Obrigatório.
    pub webhook_segredo: Segredo,
    /// Aceitar mensagem com `fromMe = true` (só teste local).
    pub aceitar_de_mim: bool,
    /// Teto de mídia em bytes (padrão [`TETO_MIDIA`]).
    pub teto_midia: u64,
}

impl ConfigCanal {
    /// Configuração com o segredo obrigatório e o teto padrão.
    pub fn novo(
        integracao: IntegrationId,
        webhook_segredo: Segredo,
    ) -> Result<Self, ErroEvolution> {
        if webhook_segredo.expor().trim().is_empty() {
            return Err(ErroEvolution::Configuracao(
                "a integração do WhatsApp precisa do segredo do webhook".to_owned(),
            ));
        }
        Ok(Self {
            integracao,
            webhook_url: None,
            webhook_segredo,
            aceitar_de_mim: false,
            teto_midia: TETO_MIDIA,
        })
    }

    /// Liga o webhook da instância para este endereço.
    pub fn com_webhook(mut self, url: impl Into<String>) -> Self {
        let url = url.into().trim().to_owned();
        self.webhook_url = if url.is_empty() { None } else { Some(url) };
        self
    }
}

/// Futuro de um comando de saída **com recibo**: o id que o canal deu à mensagem, quando deu.
///
/// Mesmo molde de `poph_types::FuturoComando`, sem `async_trait`, e separado dele de propósito: o
/// contrato do canal não muda (plugin em processo externo pode não devolver id).
pub type FuturoRecibo =
    Pin<Box<dyn Future<Output = Result<Option<String>, ErroIntegracao>> + Send + 'static>>;

/// De onde sai o conteúdo de um arquivo da biblioteca do agente.
///
/// É trait para o teste não precisar de disco e para o servidor decidir onde ficam os blobs.
pub trait FonteDeMidia: Send + Sync + 'static {
    /// Lê o conteúdo do blob (`<org>/<sha256>`).
    fn ler(&self, blob_ref: &str) -> Result<Vec<u8>, ErroEvolution>;
}

/// Blobs em disco, no mesmo formato do `poph-files` (`<raiz>/<org>/<sha256>`).
#[derive(Clone, Debug)]
pub struct BlobsEmDisco {
    raiz: PathBuf,
}

impl BlobsEmDisco {
    /// Fonte que lê de `raiz`.
    pub fn nova(raiz: impl Into<PathBuf>) -> Self {
        Self { raiz: raiz.into() }
    }
}

impl FonteDeMidia for BlobsEmDisco {
    fn ler(&self, blob_ref: &str) -> Result<Vec<u8>, ErroEvolution> {
        let relativo = Path::new(blob_ref);
        let seguro = !relativo.is_absolute()
            && relativo
                .components()
                .all(|componente| matches!(componente, Component::Normal(_)));
        if !seguro {
            return Err(ErroEvolution::BlobIlegivel {
                blob: blob_ref.to_owned(),
                motivo: "referência de blob fora da raiz".to_owned(),
            });
        }
        let caminho = self.raiz.join(relativo);
        std::fs::read(&caminho).map_err(|erro| ErroEvolution::BlobIlegivel {
            blob: blob_ref.to_owned(),
            motivo: erro.to_string(),
        })
    }
}

/// O canal WhatsApp (Evolution) visto pelo motor do turno.
#[derive(Clone)]
pub struct WhatsAppEvolution {
    cliente: ClienteEvolution,
    config: ConfigCanal,
    fonte: Arc<dyn FonteDeMidia>,
    manifesto: Arc<Manifesto>,
    estado: Arc<Mutex<EstadoIntegracao>>,
}

impl WhatsAppEvolution {
    /// Monta o canal. O estado começa em [`EstadoIntegracao::Iniciando`] até [`Self::iniciar`].
    pub fn novo(
        cliente: ClienteEvolution,
        config: ConfigCanal,
        fonte: Arc<dyn FonteDeMidia>,
    ) -> Self {
        Self {
            cliente,
            config,
            fonte,
            manifesto: Arc::new(manifesto()),
            estado: Arc::new(Mutex::new(EstadoIntegracao::Iniciando)),
        }
    }

    /// Canal só para **enviar** (a saída do servidor, HR-26c).
    ///
    /// O segredo do webhook fica com quem recebe as entregas (o servidor), então aqui ele vai
    /// vazio e não é conferido em nada. Quem usa este canal não chama `receber_webhook`.
    pub fn para_saida(
        cliente: ClienteEvolution,
        integracao: IntegrationId,
        fonte: Arc<dyn FonteDeMidia>,
    ) -> Self {
        let config = ConfigCanal {
            integracao,
            webhook_url: None,
            webhook_segredo: Segredo::new(String::new()),
            aceitar_de_mim: false,
            teto_midia: TETO_MIDIA,
        };
        Self::novo(cliente, config, fonte)
    }

    /// Confere a conexão da instância e aponta o webhook para cá, quando o endereço foi informado.
    ///
    /// Devolve o estado do canal: `open` deixa o canal `Pronto`; qualquer outra coisa o deixa
    /// `Iniciando` e o operador pareia pelo Console (QR).
    pub async fn iniciar(&self) -> Result<String, ErroEvolution> {
        let estado = self.cliente.estado_conexao().await?;
        {
            let mut guarda = self.estado.lock().expect("estado do canal");
            *guarda = if estado == "open" {
                EstadoIntegracao::Pronto
            } else {
                EstadoIntegracao::Iniciando
            };
        }
        if let Some(url) = self.config.webhook_url.as_deref() {
            self.cliente
                .definir_webhook(
                    url,
                    &["MESSAGES_UPSERT", "CONNECTION_UPDATE", "QRCODE_UPDATED"],
                    &[(
                        CABECALHO_SEGREDO.to_owned(),
                        self.config.webhook_segredo.expor().clone(),
                    )],
                )
                .await?;
        }
        Ok(estado)
    }

    /// Marca o canal como pronto/parado a partir do que o webhook contou.
    pub fn aplicar_estado(&self, estado: &poph_types::EventoEstado) {
        let mut guarda = self.estado.lock().expect("estado do canal");
        *guarda = match estado {
            poph_types::EventoEstado::Conectado { .. } => EstadoIntegracao::Pronto,
            poph_types::EventoEstado::Desconectado { .. } => EstadoIntegracao::Iniciando,
            poph_types::EventoEstado::Qr { .. } => EstadoIntegracao::Iniciando,
            poph_types::EventoEstado::Erro { .. } => EstadoIntegracao::Iniciando,
        };
    }

    /// Autentica e lê o corpo do webhook desta integração.
    pub fn receber_webhook(
        &self,
        segredo_recebido: Option<&str>,
        corpo: &serde_json::Value,
    ) -> Result<Decisao, ErroWebhook> {
        conferir_segredo(&self.config.webhook_segredo, segredo_recebido)?;
        let politica = Politica {
            aceitar_de_mim: self.config.aceitar_de_mim,
        };
        let decisao = interpretar(self.config.integracao, corpo, &politica);
        match &decisao {
            Decisao::Estado(estado) => self.aplicar_estado(estado),
            Decisao::Entrada(_) => {
                let mut guarda = self.estado.lock().expect("estado do canal");
                *guarda = EstadoIntegracao::Pronto;
            }
            Decisao::Ignorado(_) => {}
        }
        Ok(decisao)
    }

    /// Baixa e devolve o conteúdo de uma mídia recebida, conferindo o teto do canal.
    pub async fn baixar_midia(
        &self,
        midia: &crate::webhook::MidiaParaBaixar,
    ) -> Result<crate::cliente::MidiaBaixada, ErroEvolution> {
        if let Some(declarado) = midia.tamanho_declarado
            && declarado > self.config.teto_midia
        {
            return Err(ErroEvolution::MidiaGrandeDemais {
                bytes: declarado,
                teto: self.config.teto_midia,
            });
        }
        self.cliente
            .baixar_midia(&midia.message_id, &midia.dados, self.config.teto_midia)
            .await
    }

    /// Entrega o evento ao receptor depois de baixar as mídias.
    ///
    /// Mídia que falha é registrada e o evento segue **sem** ela: perder a legenda porque o arquivo
    /// de 8 MB não veio seria pior do que o cliente receber resposta sem o anexo.
    pub async fn receber_e_entregar(&self, decisao: Decisao, receptor: &dyn poph_types::Receptor) {
        match decisao {
            Decisao::Estado(estado) => {
                receptor.receber(poph_types::EventoIntegracao::Estado(estado))
            }
            Decisao::Entrada(entrada) => {
                let mut anexos = Vec::new();
                for midia in &entrada.midias {
                    match self.baixar_midia(midia).await {
                        Ok(baixada) => anexos.push(crate::webhook::anexo_embutido(
                            &midia.nome,
                            &baixada.mime,
                            &baixada.bytes,
                        )),
                        Err(erro) => tracing::warn!(
                            target: "poph_evolution",
                            message_id = %midia.message_id,
                            motivo = %erro,
                            "mídia do WhatsApp não baixada; o evento segue sem o anexo"
                        ),
                    }
                }
                let evento: InboundEvent = entrada.com_anexos(anexos);
                receptor.receber(poph_types::EventoIntegracao::Entrada(evento))
            }
            Decisao::Ignorado(motivo) => tracing::debug!(
                target: "poph_evolution",
                motivo = ?motivo,
                "entrega do WhatsApp ignorada"
            ),
        }
    }

    /// Entrega um comando de saída e devolve o **id que a Evolution deu à mensagem** (MX-40).
    ///
    /// É o mesmo caminho de [`Integration::enviar_texto`] e [`Integration::enviar_arquivo`], com o
    /// id de fora: o contrato do canal (`FuturoComando`) não carrega id porque ele serve também a
    /// plugin em processo externo, que pode não devolver nenhum. Quem precisa do id — a saída do
    /// WhatsApp, para gravar na `outbox` — chama por aqui; quem só quer entregar usa o contrato.
    ///
    /// Presença devolve `None`: a Evolution não devolve id para "digitando", e a chamada segue a
    /// mesma regra de antes (falha de presença não derruba a mensagem).
    pub fn enviar_com_recibo(&self, comando: ComandoSaida) -> FuturoRecibo {
        let this = self.clone();
        Box::pin(async move {
            match comando {
                ComandoSaida::EnviarTexto(c) => this.enviar_texto_recibo(c).await,
                ComandoSaida::EnviarArquivo(c) => this.enviar_arquivo_recibo(c).await,
                ComandoSaida::DefinirPresenca(c) => {
                    this.enviar_presenca_sem_erro(c).await.map(|()| None)
                }
            }
        })
    }

    /// Texto ao cliente, devolvendo o id da mensagem quando a Evolution manda um.
    pub async fn enviar_texto_recibo(
        &self,
        comando: SendText,
    ) -> Result<Option<String>, ErroIntegracao> {
        self.cliente
            .enviar_texto(&comando.peer, &comando.texto)
            .await
            .map_err(|erro| erro.para_integracao("enviar_texto"))
    }

    /// Arquivo ao cliente, devolvendo o id da mensagem quando a Evolution manda um.
    pub async fn enviar_arquivo_recibo(
        &self,
        comando: SendFile,
    ) -> Result<Option<String>, ErroIntegracao> {
        // Recusa antes de ler o blob: arquivo acima do teto não sai, e nem chega à memória.
        if comando.tamanho_bytes > self.config.teto_midia {
            return Err(ErroIntegracao::MensagemGrandeDemais {
                bytes: usize::try_from(comando.tamanho_bytes).unwrap_or(usize::MAX),
                teto: usize::try_from(self.config.teto_midia).unwrap_or(usize::MAX),
            });
        }
        let bytes = self
            .fonte
            .ler(&comando.blob_ref)
            .map_err(|erro| erro.para_integracao("enviar_arquivo"))?;
        self.cliente
            .enviar_midia(
                &comando.peer,
                &bytes,
                &comando.mime,
                &comando.nome,
                comando.legenda.as_deref(),
            )
            .await
            .map_err(|erro| erro.para_integracao("enviar_arquivo"))
    }

    /// "Digitando"/"pausado" sem derrubar nada: falha de presença não é falha de conteúdo.
    async fn enviar_presenca_sem_erro(&self, comando: SetPresence) -> Result<(), ErroIntegracao> {
        let digitando = matches!(comando.estado, Presenca::Digitando);
        match self.cliente.enviar_presenca(&comando.peer, digitando).await {
            Ok(()) => Ok(()),
            // Presença não é conteúdo: se a Evolution não aceitar, a mensagem ainda sai.
            Err(erro) => {
                tracing::debug!(
                    target: "poph_evolution",
                    motivo = %erro,
                    "presença não aceita pela Evolution"
                );
                Ok(())
            }
        }
    }
}

impl Integration for WhatsAppEvolution {
    fn manifesto(&self) -> Option<Manifesto> {
        Some((*self.manifesto).clone())
    }

    fn estado(&self) -> EstadoIntegracao {
        self.estado.lock().expect("estado do canal").clone()
    }

    fn enviar_texto(&self, comando: SendText) -> FuturoComando {
        let this = self.clone();
        Box::pin(async move { this.enviar_texto_recibo(comando).await.map(|_| ()) })
    }

    fn enviar_arquivo(&self, comando: SendFile) -> FuturoComando {
        let this = self.clone();
        Box::pin(async move { this.enviar_arquivo_recibo(comando).await.map(|_| ()) })
    }

    fn definir_presenca(&self, comando: SetPresence) -> FuturoComando {
        let this = self.clone();
        Box::pin(async move { this.enviar_presenca_sem_erro(comando).await })
    }

    fn saude(&self) -> FuturoSaude {
        let this = self.clone();
        Box::pin(async move {
            match this.cliente.estado_conexao().await {
                Ok(estado) if estado == "open" => Ok(Saude::ok(format!(
                    "Evolution: {estado} (instância {})",
                    this.cliente.instancia()
                ))),
                Ok(estado) => Ok(Saude {
                    ok: false,
                    detalhe: Some(format!("Evolution: {estado}")),
                }),
                Err(erro) => Err(erro.para_integracao("saude")),
            }
        })
    }
}

/// O que a integração é e como se configura (é o mesmo manifesto que o Console desenha).
fn manifesto() -> Manifesto {
    Manifesto {
        nome: "WhatsApp (Evolution)".to_owned(),
        kind: "whatsapp".to_owned(),
        versao: env!("CARGO_PKG_VERSION").to_owned(),
        protocolo: PROTOCOLO_VERSAO,
        capacidades: Capacidades {
            text: true,
            image_in: true,
            file_in: true,
            file_out: true,
            typing: true,
            read_receipt: false,
        },
        limites: LimitesCanal {
            max_anexo_bytes: TETO_MIDIA,
            max_texto_bytes: LIMITE_TEXTO as u64,
            tipos_aceitos: vec![
                "image/*".to_owned(),
                "application/pdf".to_owned(),
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                    .to_owned(),
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".to_owned(),
                "application/vnd.openxmlformats-officedocument.presentationml.presentation"
                    .to_owned(),
            ],
        },
        configuracao: EsquemaJson(serde_json::json!({
            "type": "object",
            "required": ["instancia"],
            "properties": {
                "instancia": {
                    "type": "string",
                    "description": "Nome da instância na EvolutionAPI (o mesmo que aparece no painel dela)."
                },
                "url_base": {
                    "type": "string",
                    "format": "uri",
                    "description": "Endereço da EvolutionAPI (http:// na rede local, https:// exposta)."
                },
                "webhook_url": {
                    "type": "string",
                    "format": "uri",
                    "description": "Endereço em que a Evolution entrega as mensagens. Vazio = não mexe no webhook já configurado."
                }
            }
        })),
        segredos: EsquemaJson(serde_json::json!({
            "type": "object",
            "required": ["apikey", "webhook_secret"],
            "properties": {
                "apikey": {
                    "type": "string",
                    "format": "password",
                    "x-secret": true,
                    "description": "Chave da EvolutionAPI (cabeçalho `apikey`)."
                },
                "webhook_secret": {
                    "type": "string",
                    "format": "password",
                    "x-secret": true,
                    "description": "Segredo conferido no cabeçalho `X-Evolution-Webhook-Secret` de cada entrega."
                }
            }
        })),
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn manifesto_declara_o_que_o_canal_faz() {
        let manifesto = manifesto();
        assert_eq!(manifesto.kind, "whatsapp");
        assert_eq!(manifesto.protocolo, PROTOCOLO_VERSAO);
        assert!(manifesto.capacidades.text);
        assert!(manifesto.capacidades.file_in);
        assert!(manifesto.capacidades.file_out);
        assert_eq!(manifesto.validar(), Ok(()));
    }

    #[test]
    fn segredo_do_webhook_e_obrigatorio() {
        let erro = ConfigCanal::novo(IntegrationId::generate(), Segredo::new(String::new()))
            .expect_err("sem segredo deve falhar");
        assert!(matches!(erro, ErroEvolution::Configuracao(_)));
    }

    #[test]
    fn blob_fora_da_raiz_e_recusado() {
        let fonte = BlobsEmDisco::nova("/tmp/blobs");
        assert!(fonte.ler("../etc/passwd").is_err());
        assert!(fonte.ler("/etc/passwd").is_err());
    }
}
