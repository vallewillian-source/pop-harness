//! Contrato de integração e protocolo de plugin (PRD §8.3, HR-25).
//!
//! Um canal do Harness (WhatsApp, plugin de terceiro) implementa [`Integration`]. O mesmo contrato
//! vale para o módulo nativo em Rust (HR-26) e para o **plugin em processo externo**, que fala
//! JSON-lines sobre stdio com as mensagens [`MensagemHost`] e [`MensagemPlugin`] — o formato está
//! descrito em `apps/pop-harness/docs/PLUGIN_PROTOCOLO.md`.
//!
//! Regras do contrato:
//!
//! - **Versão explícita.** [`PROTOCOLO_VERSAO`] vai no handshake dos dois lados e uma diferença é
//!   recusada com [`ErroIntegracao::ProtocoloIncompativel`] — nunca se adivinha o formato.
//! - **Segredos só no handshake.** Eles viajam em [`Handshake::segredos`] e em mais nada: nem no
//!   evento de entrada, nem no comando de saída, nem no erro. O `Debug` do handshake redige a lista
//!   (os tipos do domínio usam `Segredo`, que não implementa `Serialize` de propósito).
//! - **Manifesto** ([`Manifesto`]) descreve capacidades, limites e os **JSON Schema** do formulário
//!   de configuração e dos segredos, que o Console desenha (HR-38).
//! - **Entrada** ([`InboundEvent`]) e **saída** ([`ComandoSaida`]) são tipos serializáveis, sem
//!   lógica: quem decide o que fazer com eles é o motor do turno e o supervisor do plugin.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ids::{IntegrationId, OrgId};

/// Versão do protocolo host ↔ plugin.
///
/// Muda quando o formato das mensagens deixa de ser compatível: campo obrigatório novo, tipo
/// trocado, mensagem removida. Campo opcional novo **não** muda a versão (o leitor ignora o que não
/// conhece). O handshake recusa versão diferente da esperada.
pub const PROTOCOLO_VERSAO: u32 = 1;

/// Teto padrão do formulário de configuração: o mesmo do arquivo enviado pelo cliente (16 MB), que
/// o manifesto usa como valor inicial quando o plugin não informa outro.
pub const LIMITE_ANEXO_PADRAO: u64 = 16 * 1024 * 1024;

// ───────────────────────────── Manifesto ─────────────────────────────

/// O que o plugin sabe fazer e como se configura (PRD §8.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifesto {
    /// Nome legível do plugin ("WhatsApp", "Eco").
    pub nome: String,
    /// Tipo da integração: `whatsapp`, `telegram`, `eco`… O mesmo `kind` da tabela `integrations`.
    pub kind: String,
    /// Versão do próprio plugin (livre; serve para o operador comparar).
    pub versao: String,
    /// Versão do protocolo que o plugin fala. Tem de ser igual a [`PROTOCOLO_VERSAO`].
    pub protocolo: u32,
    /// O que o canal consegue fazer.
    #[serde(default)]
    pub capacidades: Capacidades,
    /// Limites que o canal aceita.
    #[serde(default)]
    pub limites: LimitesCanal,
    /// JSON Schema do formulário de configuração (o Console desenha a partir dele).
    pub configuracao: EsquemaJson,
    /// JSON Schema dos segredos que o operador preenche. Vazio = o canal não usa segredo.
    #[serde(default)]
    pub segredos: EsquemaJson,
}

impl Manifesto {
    /// Confere o que, se estiver errado, faz o handshake falhar.
    ///
    /// A versão de protocolo é conferida pelo supervisor antes (a mensagem de erro diz as duas
    /// versões); aqui ficam os campos que só o manifesto conhece.
    pub fn validar(&self) -> Result<(), ErroIntegracao> {
        if self.nome.trim().is_empty() {
            return Err(ErroIntegracao::HandshakeRecusado {
                motivo: "o manifesto não traz o nome do plugin".to_owned(),
            });
        }
        if self.kind.trim().is_empty() {
            return Err(ErroIntegracao::HandshakeRecusado {
                motivo: "o manifesto não traz o tipo (`kind`) da integração".to_owned(),
            });
        }
        if self.configuracao.0.is_null() {
            return Err(ErroIntegracao::HandshakeRecusado {
                motivo: "o manifesto não traz o JSON Schema da configuração".to_owned(),
            });
        }
        self.configuracao.validar("configuração")?;
        if !self.segredos.0.is_null() {
            self.segredos.validar("segredos")?;
        }
        Ok(())
    }
}

/// Capacidades do canal (PRD §8.3). Ausente = `false`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capacidades {
    /// Envia e recebe texto.
    #[serde(default)]
    pub text: bool,
    /// Recebe imagem do cliente.
    #[serde(default)]
    pub image_in: bool,
    /// Recebe arquivo do cliente (PDF, Office).
    #[serde(default)]
    pub file_in: bool,
    /// Envia arquivo da biblioteca do agente.
    #[serde(default)]
    pub file_out: bool,
    /// Mostra "digitando".
    #[serde(default)]
    pub typing: bool,
    /// Informa leitura da mensagem.
    #[serde(default)]
    pub read_receipt: bool,
}

/// Limites do canal, usados para recusar antes de enviar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LimitesCanal {
    /// Maior arquivo aceito, em bytes.
    #[serde(default = "limite_anexo_padrao")]
    pub max_anexo_bytes: u64,
    /// Maior texto aceito, em bytes (0 = sem teto declarado).
    #[serde(default)]
    pub max_texto_bytes: u64,
    /// Tipos MIME aceitos (vazio = o plugin não declara restrição).
    #[serde(default)]
    pub tipos_aceitos: Vec<String>,
}

fn limite_anexo_padrao() -> u64 {
    LIMITE_ANEXO_PADRAO
}

impl Default for LimitesCanal {
    fn default() -> Self {
        Self {
            max_anexo_bytes: LIMITE_ANEXO_PADRAO,
            max_texto_bytes: 0,
            tipos_aceitos: Vec::new(),
        }
    }
}

/// JSON Schema (draft 2020-12 em diante) do formulário de configuração ou dos segredos.
///
/// O Harness não implementa validação de JSON Schema: ele guarda o documento e o entrega ao
/// Console, que desenha o formulário. O que se confere aqui é só a forma mínima — um objeto com
/// `type: "object"` — para um manifesto quebrado não virar formulário quebrado.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EsquemaJson(pub serde_json::Value);

impl EsquemaJson {
    /// Esquema vazio (nenhuma configuração, nenhum segredo).
    pub fn vazio() -> Self {
        Self(serde_json::json!({"type": "object", "properties": {}}))
    }

    /// Confere que é um esquema de objeto com `properties` objeto.
    pub fn validar(&self, rotulo: &'static str) -> Result<(), ErroIntegracao> {
        let mut motivo = None;
        match self.0.as_object() {
            None => motivo = Some("não é um objeto JSON"),
            Some(objeto) => {
                if objeto.get("type").and_then(|t| t.as_str()) != Some("object") {
                    motivo = Some("`type` não é \"object\"");
                } else if let Some(propriedades) = objeto.get("properties")
                    && !propriedades.is_object()
                {
                    motivo = Some("`properties` não é um objeto");
                }
            }
        }
        match motivo {
            None => Ok(()),
            Some(motivo) => Err(ErroIntegracao::HandshakeRecusado {
                motivo: format!("o JSON Schema de {rotulo} é inválido: {motivo}"),
            }),
        }
    }
}

impl Default for EsquemaJson {
    fn default() -> Self {
        Self::vazio()
    }
}

// ───────────────────────────── Plugins declarados ─────────────────────────────

/// Plugins de integração que o servidor conhece (HR-44c).
///
/// O manifesto de um plugin é a única fonte do formulário de configuração e da lista de segredos
/// que o operador preenche. Ele só existe no handshake — por isso o servidor precisa saber **qual
/// programa** executar para descobrir o manifesto de cada `kind`. Esta seção é essa declaração:
/// sem ela, a API recusa `config_json` (não há esquema para validar) e a rota de manifesto responde
/// que o tipo não está declarado.
///
/// O programa é caminho **local** do servidor, nunca vem de requisição: o manifesto é lido subindo
/// o plugin uma vez e guardando o resultado em memória (ver `poph-server::manifestos`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct PluginsConfig {
    /// Um item por `kind` de plugin. `kind` repetido é erro de configuração.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub integracoes: Vec<PluginDeclarado>,
}

impl PluginsConfig {
    /// `true` quando não há plugin declarado (a seção é omitida ao serializar).
    pub fn vazio(&self) -> bool {
        self.integracoes.is_empty()
    }
}

/// Um plugin declarado: o tipo da integração e o programa que responde ao handshake.
///
/// A seção não guarda segredo nem configuração de integração: o que o operador preenche fica na
/// tabela `integrations` (segredo cifrado). Aqui mora só o endereço do binário.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginDeclarado {
    /// Tipo da integração que o plugin atende (o mesmo `kind` da tabela `integrations`).
    pub kind: String,
    /// Programa que fala o protocolo de plugin (caminho absoluto ou nome no `PATH`).
    pub programa: PathBuf,
    /// Argumentos do programa. **Nunca** ponha segredo aqui: eles aparecem em `ps` e no `/proc`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub argumentos: Vec<String>,
}

// ───────────────────────────── Entrada normalizada ─────────────────────────────

/// Evento de entrada normalizado: o que o canal entrega ao Harness (PRD §8.3).
///
/// Os nomes dos campos são o contrato do protocolo (e o formato que a `inbox` vai receber na
/// HR-26); quem grava a mensagem é o fluxo da HR-12, a partir daqui.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InboundEvent {
    /// Integração que recebeu.
    pub integracao: IntegrationId,
    /// Id da mensagem no canal — é a chave de idempotência da entrega.
    pub id_externo: String,
    /// Contato/canal do outro lado (telefone, jid, sala).
    pub peer: String,
    /// Quem escreveu, quando difere do `peer` (grupo, por exemplo).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remetente: Option<String>,
    /// Texto recebido (vazio quando é só anexo).
    #[serde(default)]
    pub texto: String,
    /// Anexos recebidos.
    #[serde(default)]
    pub anexos: Vec<AnexoEntrada>,
    /// Quando o canal recebeu a mensagem (ms desde a época).
    pub ocorrido_em: i64,
    /// Dados do canal que não têm campo próprio (id de resposta, menções…).
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub metadados: serde_json::Value,
}

impl InboundEvent {
    /// Confere o mínimo que o motor do turno precisa para processar o evento.
    pub fn validar(&self) -> Result<(), ErroIntegracao> {
        if self.id_externo.trim().is_empty() {
            return Err(ErroIntegracao::Protocolo {
                motivo: "evento de entrada sem `id_externo`".to_owned(),
            });
        }
        if self.peer.trim().is_empty() {
            return Err(ErroIntegracao::Protocolo {
                motivo: "evento de entrada sem `peer`".to_owned(),
            });
        }
        if self.ocorrido_em <= 0 {
            return Err(ErroIntegracao::Protocolo {
                motivo: "evento de entrada sem `ocorrido_em` válido".to_owned(),
            });
        }
        for anexo in &self.anexos {
            anexo.validar()?;
        }
        Ok(())
    }
}

/// Anexo do cliente: ou já está no armazenamento do Harness (`blob`), ou vem no próprio evento em
/// base64, com o limite do canal (`bytes`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "fonte", rename_all = "snake_case")]
pub enum AnexoEntrada {
    /// Blob já gravado: o plugin só diz qual é.
    Blob {
        /// Nome original.
        nome: String,
        /// Tipo MIME informado pelo canal.
        mime: String,
        /// sha256 do conteúdo (64 hexa), como em `data/blobs/<org>/<sha256>`.
        sha256: String,
        /// Tamanho em bytes.
        tamanho_bytes: u64,
    },
    /// Bytes no próprio evento, em base64 (só para canais que não têm armazenamento próprio).
    Bytes {
        /// Nome original.
        nome: String,
        /// Tipo MIME informado pelo canal.
        mime: String,
        /// Conteúdo em base64.
        dados_base64: String,
    },
}

impl AnexoEntrada {
    fn validar(&self) -> Result<(), ErroIntegracao> {
        let (nome, mime) = match self {
            AnexoEntrada::Blob {
                nome,
                mime,
                sha256,
                tamanho_bytes,
            } => {
                if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(ErroIntegracao::Protocolo {
                        motivo: format!("anexo `{nome}` com sha256 inválido (esperado 64 hexa)"),
                    });
                }
                if *tamanho_bytes == 0 {
                    return Err(ErroIntegracao::Protocolo {
                        motivo: format!("anexo `{nome}` com tamanho zero"),
                    });
                }
                (nome, mime)
            }
            AnexoEntrada::Bytes {
                nome,
                mime,
                dados_base64,
            } => {
                if dados_base64.is_empty() {
                    return Err(ErroIntegracao::Protocolo {
                        motivo: format!("anexo `{nome}` sem conteúdo"),
                    });
                }
                (nome, mime)
            }
        };
        if nome.trim().is_empty() || mime.trim().is_empty() {
            return Err(ErroIntegracao::Protocolo {
                motivo: "anexo sem nome ou sem tipo MIME".to_owned(),
            });
        }
        Ok(())
    }
}

// ───────────────────────────── Saída (comandos) ─────────────────────────────

/// Comando de saída: o que o Harness manda o canal entregar (PRD §8.3).
///
/// `idempotencia` vem do `(turn_id, seq)` da `outbox` (HR-12): o canal que reenviar depois de uma
/// queda deve usar a mesma chave para não duplicar quando souber fazer isso (D11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tipo", rename_all = "snake_case")]
pub enum ComandoSaida {
    /// Enviar texto.
    EnviarTexto(SendText),
    /// Enviar arquivo da biblioteca do agente.
    EnviarArquivo(SendFile),
    /// Mostrar "digitando"/"pausado".
    DefinirPresenca(SetPresence),
}

impl ComandoSaida {
    /// Nome do comando no protocolo (`enviar_texto`, `enviar_arquivo`, `definir_presenca`).
    pub fn nome(&self) -> &'static str {
        match self {
            ComandoSaida::EnviarTexto(_) => "enviar_texto",
            ComandoSaida::EnviarArquivo(_) => "enviar_arquivo",
            ComandoSaida::DefinirPresenca(_) => "definir_presenca",
        }
    }
}

/// Texto a enviar ao cliente.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendText {
    /// Chave de idempotência (`turn_id:seq`).
    pub idempotencia: String,
    /// Destino (o mesmo `peer` do evento de entrada).
    pub peer: String,
    /// Texto.
    pub texto: String,
}

/// Arquivo a enviar (só da biblioteca do agente — D13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendFile {
    /// Chave de idempotência (`turn_id:seq`).
    pub idempotencia: String,
    /// Destino.
    pub peer: String,
    /// Nome que o cliente vê.
    pub nome: String,
    /// Tipo MIME.
    pub mime: String,
    /// Tamanho em bytes.
    pub tamanho_bytes: u64,
    /// sha256 do conteúdo.
    pub sha256: String,
    /// Referência do blob (`<org>/<sha256>`), como em `data/blobs/`.
    pub blob_ref: String,
    /// Legenda opcional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legenda: Option<String>,
}

/// Presença ("digitando").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Presenca {
    /// Está digitando.
    Digitando,
    /// Parou de digitar.
    Pausado,
}

/// Presença a definir.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetPresence {
    /// Destino.
    pub peer: String,
    /// Estado.
    pub estado: Presenca,
}

// ───────────────────────────── Estado e saúde ─────────────────────────────

/// Evento de estado do canal (PRD §8.3: conectado, QR, erro).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "estado", rename_all = "snake_case")]
pub enum EventoEstado {
    /// Canal conectado.
    Conectado {
        /// Desde quando (ms).
        #[serde(default)]
        desde: i64,
    },
    /// Canal desconectado.
    Desconectado {
        /// Por quê, quando o canal sabe.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        motivo: Option<String>,
    },
    /// QR para o operador parear o aparelho.
    Qr {
        /// Conteúdo do QR (texto que o Console desenha).
        codigo: String,
        /// Quando expira (ms), quando o canal sabe.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expira_em: Option<i64>,
    },
    /// O canal reportou um erro próprio.
    Erro {
        /// Código curto (`401`, `media_too_large`).
        codigo: String,
        /// Mensagem legível.
        mensagem: String,
        /// `true` = dá para retentar.
        #[serde(default)]
        transitorio: bool,
    },
}

/// Resposta de `health` (PRD §8.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saude {
    /// `true` = o canal está operando.
    pub ok: bool,
    /// Detalhe legível (versão do canal, motivo de estar fora).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detalhe: Option<String>,
}

impl Saude {
    /// Canal saudável com um detalhe.
    pub fn ok(detalhe: impl Into<String>) -> Self {
        Self {
            ok: true,
            detalhe: Some(detalhe.into()),
        }
    }
}

/// Erro que o plugin devolveu para um comando.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErroComando {
    /// Código curto.
    pub codigo: String,
    /// Mensagem legível.
    pub mensagem: String,
    /// `true` = vale retentar.
    #[serde(default)]
    pub transitorio: bool,
}

// ───────────────────────────── Mensagens do protocolo ─────────────────────────────

/// Primeira mensagem do host: abre o protocolo, entrega configuração e **segredos**.
///
/// Os segredos saem do ambiente e da linha de comando do processo (PRD §8.3): o único caminho é
/// este campo. `Debug` redige a lista, para o handshake nunca aparecer inteiro num log.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Handshake {
    /// Versão do protocolo que o host fala.
    pub protocolo: u32,
    /// Integração configurada.
    pub integracao: IntegrationId,
    /// Org dona da integração.
    pub org: OrgId,
    /// Tipo da integração (o mesmo do manifesto).
    pub kind: String,
    /// Configuração preenchida pelo operador (validada contra o JSON Schema do manifesto).
    #[serde(default)]
    pub config: serde_json::Value,
    /// Segredos do canal. Nunca aparecem em evento, comando, erro ou log.
    #[serde(default)]
    pub segredos: BTreeMap<String, String>,
}

impl fmt::Debug for Handshake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Handshake")
            .field("protocolo", &self.protocolo)
            .field("integracao", &self.integracao)
            .field("org", &self.org)
            .field("kind", &self.kind)
            .field("config", &self.config)
            .field(
                "segredos",
                &format_args!("{} chave(s) [oculto]", self.segredos.len()),
            )
            .finish()
    }
}

/// Mensagem do host para o plugin (uma por linha).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "tipo", rename_all = "snake_case")]
pub enum MensagemHost {
    /// Primeira mensagem: protocolo, configuração e segredos.
    Handshake(Handshake),
    /// Comando de saída, com `id` para correlacionar a resposta.
    Comando {
        /// Id do pedido (único na conexão).
        id: u64,
        /// O que entregar.
        comando: ComandoSaida,
    },
    /// Pedido de saúde.
    Saude {
        /// Id do pedido.
        id: u64,
    },
    /// Encerrar o processo (o host mata se não sair).
    Encerrar,
}

/// Mensagem do plugin para o host (uma por linha).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "tipo", rename_all = "snake_case")]
pub enum MensagemPlugin {
    /// Resposta ao handshake, com o manifesto.
    HandshakeOk {
        /// Versão do protocolo que o plugin fala.
        protocolo: u32,
        /// O que o plugin faz e como se configura.
        manifesto: Manifesto,
    },
    /// Evento de entrada normalizado (mensagem do cliente, arquivo, etc.).
    Evento {
        /// O evento.
        evento: InboundEvent,
    },
    /// Evento de estado do canal.
    Estado {
        /// O estado.
        estado: EventoEstado,
    },
    /// Resposta a um comando.
    Resultado {
        /// Id do comando respondido.
        id: u64,
        /// `true` = entregue.
        ok: bool,
        /// Motivo, quando `ok = false`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        erro: Option<ErroComando>,
    },
    /// Resposta ao pedido de saúde.
    Saude {
        /// Id do pedido respondido.
        id: u64,
        /// Saúde do canal.
        saude: Saude,
    },
    /// Erro não solicitado (o canal reclamou de algo).
    Erro {
        /// Código curto.
        codigo: String,
        /// Mensagem legível.
        mensagem: String,
        /// `true` = vale retentar.
        #[serde(default)]
        transitorio: bool,
    },
}

// ───────────────────────────── Trait e erros ─────────────────────────────

/// Futuro de um comando de saída (sem `async_trait`, como no transporte da HR-12).
pub type FuturoComando = Pin<Box<dyn Future<Output = Result<(), ErroIntegracao>> + Send + 'static>>;

/// Futuro do pedido de saúde.
pub type FuturoSaude =
    Pin<Box<dyn Future<Output = Result<Saude, ErroIntegracao>> + Send + 'static>>;

/// Estado do vínculo entre o Harness e o canal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EstadoIntegracao {
    /// Processo subindo ou handshake em andamento.
    Iniciando,
    /// Handshake feito: dá para enviar.
    Pronto,
    /// Caiu e vai voltar depois da espera.
    Reiniciando {
        /// Falhas seguidas até aqui.
        falhas: u32,
        /// Quanto falta esperar (ms).
        espera_ms: u64,
    },
    /// Passou do teto de falhas: não reinicia mais sozinho.
    Caida {
        /// Falhas seguidas.
        falhas: u32,
        /// Último motivo.
        motivo: String,
    },
    /// Encerrada por quem chamou.
    Encerrada,
}

impl EstadoIntegracao {
    /// `true` quando o canal está aceitando comandos.
    pub fn esta_pronta(&self) -> bool {
        matches!(self, EstadoIntegracao::Pronto)
    }

    /// `true` quando nada mais vai acontecer sem intervenção.
    pub fn esta_encerrada(&self) -> bool {
        matches!(
            self,
            EstadoIntegracao::Caida { .. } | EstadoIntegracao::Encerrada
        )
    }
}

/// Quem recebe os eventos que a integração produz.
///
/// O motor do turno (na HR-26) transforma [`InboundEvent`] em `MensagemRecebida` da `inbox`; o
/// Console usa os [`EventoEstado`] para mostrar "conectado"/QR. A chamada é síncrona de propósito:
/// quem recebe deve enfileirar o evento, não segurar o plugin.
pub trait Receptor: Send + Sync + 'static {
    /// Recebe um evento de entrada ou de estado.
    fn receber(&self, evento: EventoIntegracao);
}

/// O que uma integração publica para quem a observa.
#[derive(Debug, Clone, PartialEq)]
pub enum EventoIntegracao {
    /// Mensagem (ou arquivo) do cliente.
    Entrada(InboundEvent),
    /// Estado do canal (conectado, QR, erro).
    Estado(EventoEstado),
}

/// Contrato de um canal (PRD §8.3). Implementado pelo módulo nativo e pelo supervisor de plugin.
pub trait Integration: Send + Sync + 'static {
    /// Manifesto recebido no handshake (vazio antes dele).
    fn manifesto(&self) -> Option<Manifesto>;

    /// Estado atual do vínculo.
    fn estado(&self) -> EstadoIntegracao;

    /// Envia texto ao cliente.
    fn enviar_texto(&self, comando: SendText) -> FuturoComando;

    /// Envia um arquivo da biblioteca do agente.
    fn enviar_arquivo(&self, comando: SendFile) -> FuturoComando;

    /// Define "digitando"/"pausado".
    fn definir_presenca(&self, comando: SetPresence) -> FuturoComando;

    /// Pergunta ao canal se ele está operando.
    fn saude(&self) -> FuturoSaude;
}

/// Tudo o que pode dar errado ao falar com um canal.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ErroIntegracao {
    /// O processo do plugin não subiu.
    #[error("não consegui iniciar o plugin `{programa}`: {motivo}")]
    FalhaAoIniciar {
        /// Programa que se tentou executar.
        programa: String,
        /// O que o sistema operacional disse.
        motivo: String,
    },

    /// Versão de protocolo diferente.
    #[error("protocolo incompatível: o host fala {esperado} e o plugin falou {recebido}")]
    ProtocoloIncompativel {
        /// Versão do host.
        esperado: u32,
        /// Versão do plugin.
        recebido: u32,
    },

    /// O plugin respondeu algo que não fecha com o contrato.
    #[error("handshake recusado: {motivo}")]
    HandshakeRecusado {
        /// O que está errado.
        motivo: String,
    },

    /// Mensagem fora do contrato (JSON inválido já é lixo, isto é contrato quebrado).
    #[error("protocolo de plugin inválido: {motivo}")]
    Protocolo {
        /// O que está errado.
        motivo: String,
    },

    /// Linha, evento ou resposta que passa do teto de tamanho.
    #[error("mensagem de {bytes} bytes passa do limite de {teto} bytes")]
    MensagemGrandeDemais {
        /// Tamanho tentado.
        bytes: usize,
        /// Teto configurado.
        teto: usize,
    },

    /// O canal não está disponível agora (caindo, reiniciando ou sem processo).
    #[error("integração indisponível: {motivo}")]
    Indisponivel {
        /// Motivo legível.
        motivo: String,
    },

    /// O plugin não respondeu dentro do prazo (o supervisor o aborta e reinicia).
    #[error("tempo esgotado em {operacao} depois de {limite_ms} ms")]
    TempoEsgotado {
        /// Operação que estourou.
        operacao: String,
        /// Prazo.
        limite_ms: u64,
    },

    /// O plugin recusou o comando.
    #[error("o plugin recusou {operacao}: {codigo} ({mensagem})")]
    Recusado {
        /// Operação recusada.
        operacao: String,
        /// Código do plugin.
        codigo: String,
        /// Mensagem do plugin.
        mensagem: String,
        /// `true` = vale retentar.
        transitoria: bool,
    },

    /// Passou do teto de falhas e não reinicia mais sozinho.
    #[error("integração caída depois de {falhas} falhas seguidas: {motivo}")]
    Caida {
        /// Falhas seguidas.
        falhas: u32,
        /// Último motivo.
        motivo: String,
    },

    /// Integração encerrada por quem chamou.
    #[error("integração encerrada")]
    Encerrada,
}

impl ErroIntegracao {
    /// `true` quando repetir a operação mais tarde pode dar certo.
    pub fn eh_transitorio(&self) -> bool {
        match self {
            ErroIntegracao::Indisponivel { .. } | ErroIntegracao::TempoEsgotado { .. } => true,
            ErroIntegracao::Recusado { transitoria, .. } => *transitoria,
            _ => false,
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn manifesto_exemplo() -> Manifesto {
        Manifesto {
            nome: "Eco".to_owned(),
            kind: "eco".to_owned(),
            versao: "1.0.0".to_owned(),
            protocolo: PROTOCOLO_VERSAO,
            capacidades: Capacidades {
                text: true,
                file_out: true,
                ..Capacidades::default()
            },
            limites: LimitesCanal::default(),
            configuracao: EsquemaJson(serde_json::json!({
                "type": "object",
                "properties": {"modo": {"type": "string"}}
            })),
            segredos: EsquemaJson(serde_json::json!({
                "type": "object",
                "properties": {"token": {"type": "string"}}
            })),
        }
    }

    #[test]
    fn manifesto_valido_passa() {
        assert_eq!(manifesto_exemplo().validar(), Ok(()));
    }

    #[test]
    fn manifesto_sem_kind_e_recusado() {
        let mut manifesto = manifesto_exemplo();
        manifesto.kind = "  ".to_owned();
        assert!(matches!(
            manifesto.validar(),
            Err(ErroIntegracao::HandshakeRecusado { .. })
        ));
    }

    #[test]
    fn esquema_que_nao_e_objeto_e_recusado() {
        let mut manifesto = manifesto_exemplo();
        manifesto.configuracao = EsquemaJson(serde_json::json!({"type": "array"}));
        let erro = manifesto.validar().unwrap_err();
        assert!(erro.to_string().contains("configuração"), "{erro}");
    }

    #[test]
    fn handshake_esconde_os_segredos_no_debug() {
        let segredo = "tok-super-secreto-123".to_owned();
        let handshake = Handshake {
            protocolo: PROTOCOLO_VERSAO,
            integracao: IntegrationId::generate(),
            org: OrgId::generate(),
            kind: "eco".to_owned(),
            config: serde_json::json!({"modo": "normal"}),
            segredos: BTreeMap::from([("token".to_owned(), segredo.clone())]),
        };
        let texto = format!("{handshake:?} {handshake:#?}");
        assert!(!texto.contains(&segredo), "debug vazou: {texto}");
        assert!(texto.contains("1 chave(s) [oculto]"), "{texto}");

        // Serializar é o caminho do envio: o valor tem de estar lá.
        let json = serde_json::to_string(&handshake).unwrap();
        assert!(json.contains(&segredo));
    }

    #[test]
    fn evento_valido_passa_e_sem_id_externo_falha() {
        let evento = InboundEvent {
            integracao: IntegrationId::generate(),
            id_externo: "abc-1".to_owned(),
            peer: "5511999999999".to_owned(),
            remetente: None,
            texto: "oi".to_owned(),
            anexos: vec![AnexoEntrada::Blob {
                nome: "manual.pdf".to_owned(),
                mime: "application/pdf".to_owned(),
                sha256: "a".repeat(64),
                tamanho_bytes: 10,
            }],
            ocorrido_em: 1_795_140_000_000,
            metadados: serde_json::Value::Null,
        };
        assert_eq!(evento.validar(), Ok(()));

        let mut sem_id = evento.clone();
        sem_id.id_externo = String::new();
        assert!(matches!(
            sem_id.validar(),
            Err(ErroIntegracao::Protocolo { .. })
        ));

        let mut sha_curto = evento;
        sha_curto.anexos = vec![AnexoEntrada::Blob {
            nome: "manual.pdf".to_owned(),
            mime: "application/pdf".to_owned(),
            sha256: "abc".to_owned(),
            tamanho_bytes: 10,
        }];
        assert!(sha_curto.validar().is_err());
    }

    #[test]
    fn mensagens_do_protocolo_tem_a_forma_documentada() {
        let comando = MensagemHost::Comando {
            id: 7,
            comando: ComandoSaida::EnviarTexto(SendText {
                idempotencia: "turno:0".to_owned(),
                peer: "5511".to_owned(),
                texto: "oi".to_owned(),
            }),
        };
        let json = serde_json::to_value(&comando).unwrap();
        assert_eq!(json["tipo"], "comando");
        assert_eq!(json["id"], 7);
        assert_eq!(json["comando"]["tipo"], "enviar_texto");
        assert_eq!(json["comando"]["texto"], "oi");

        let resposta: MensagemPlugin = serde_json::from_str(
            r#"{"tipo":"resultado","id":7,"ok":false,"erro":{"codigo":"401","mensagem":"sessão caiu","transitorio":false}}"#,
        )
        .unwrap();
        assert!(matches!(
            resposta,
            MensagemPlugin::Resultado { ok: false, .. }
        ));

        // Campo desconhecido não quebra: é assim que o protocolo cresce sem mudar de versão.
        let futuro: MensagemPlugin =
            serde_json::from_str(r#"{"tipo":"resultado","id":1,"ok":true,"extra":42}"#).unwrap();
        assert!(matches!(futuro, MensagemPlugin::Resultado { ok: true, .. }));
    }

    #[test]
    fn erro_transitorio_e_reconhecido() {
        assert!(
            ErroIntegracao::TempoEsgotado {
                operacao: "comando".to_owned(),
                limite_ms: 10
            }
            .eh_transitorio()
        );
        assert!(
            !ErroIntegracao::ProtocoloIncompativel {
                esperado: 1,
                recebido: 2
            }
            .eh_transitorio()
        );
        assert!(
            ErroIntegracao::Recusado {
                operacao: "enviar_texto".to_owned(),
                codigo: "429".to_owned(),
                mensagem: "devagar".to_owned(),
                transitoria: true,
            }
            .eh_transitorio()
        );
    }
}
