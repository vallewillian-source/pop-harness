//! Ligação entre as filas (`inbox`/`outbox`) e o contrato de integração (HR-27).
//!
//! O contrato do canal (`trait Integration`, `InboundEvent`, `ComandoSaida`) mora em
//! [`poph_types::integracao`] e o supervisor de plugin em processo externo em `poph-plugins`. O
//! que faltava era o encaixe com as filas da HR-12:
//!
//! - [`EnvioPeloCanal`] implementa [`TransporteSaida`]: pega o item pendente da `outbox` e o
//!   entrega como [`ComandoSaida`] ao canal, seja ele o WhatsApp nativo (HR-26) ou um plugin em
//!   processo externo (HR-25). O item só vira `sent` depois que o canal **confirma**.
//! - [`InboxDoCanal`] implementa [`Receptor`]: recebe o [`InboundEvent`] do canal e grava na
//!   `inbox` pelo caminho da HR-12, com a idempotência por `(integração, id externo)`.
//!
//! Por que aqui e não em `poph-plugins`: este crate é quem conhece a `outbox`/`inbox` ([`Fluxo`])
//! e o [`TransporteSaida`]; o `poph-plugins` fica só com o protocolo e o supervisor, sem arrastar
//! SQLite, RAG e o resto do motor (ele continua no check aarch64 de código puro).
//!
//! ```no_run
//! # async fn exemplo() -> Result<(), Box<dyn std::error::Error>> {
//! use std::sync::Arc;
//! use poph_agent::entrada_saida::{Fluxo, TransporteSaida};
//! use poph_agent::fila_integracao::{ConversaDoCanal, EnvioPeloCanal, InboxDoCanal};
//! # let fluxo: Fluxo = unimplemented!();
//! # let org = unimplemented!();
//! # let agente = unimplemented!();
//! # let canal: Arc<dyn poph_types::Integration> = unimplemented!();
//! // saída: outbox → canal
//! let envio = EnvioPeloCanal::novo(Arc::clone(fluxo.store()), Arc::clone(&canal));
//! fluxo.enviar_pendentes(org, &envio).await?;
//! // entrada: canal → inbox
//! let resolvedor = Arc::new(ConversaDoCanal::novo(
//!     Arc::clone(fluxo.store()),
//!     agente,
//!     "eco",
//!     None,
//! ));
//! let entrada = InboxDoCanal::iniciar(fluxo.clone(), org, resolvedor);
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use poph_store::{Conversa, ItemSaida, NovaConversa, Pagina, Recebido, StatusConversa, Store};
use poph_types::{
    AgentId, ComandoSaida, ConversationId, ErroIntegracao, EventoEstado, EventoIntegracao,
    InboundEvent, Integration, IntegrationId, OrgId, Presenca, Receptor, SendFile, SendText,
    SetPresence,
};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::entrada_saida::{FalhaEnvio, Fluxo, FuturoEnvio, MensagemRecebida, TransporteSaida};
use crate::erro::Resultado;

/// Quantos eventos do canal cabem na fila antes de o receptor recusar (contrapressão).
///
/// O supervisor do plugin já limita a 256 linhas o que ele lê do processo; esta fila é a segunda
/// barreira: se a gravação no banco ficar lenta, a recusa aparece em
/// [`RelatoEntrada::recusados`] em vez de crescer em memória.
const FILA_EVENTOS: usize = 1024;

/// Quantas páginas de conversa a busca por par (canal + peer) percorre antes de abrir uma nova.
const PAGINAS_BUSCA: u32 = 50;

/// Teto do cache de conversa por contato (cada conversa ativa ocupa uma entrada).
const TETO_CACHE: usize = 4096;

// ───────────────────────────── Saída: outbox → canal ─────────────────────────────

/// Transporte de saída que entrega o item da `outbox` ao canal configurado.
///
/// Serve para qualquer [`Integration`]: o supervisor de plugin (HR-25), o WhatsApp nativo (HR-26)
/// ou outra integração futura. A tradução item → comando usa a mesma chave de idempotência do
/// protocolo (`turn_id:seq`).
pub struct EnvioPeloCanal {
    store: Arc<Store>,
    canal: Arc<dyn Integration>,
}

impl EnvioPeloCanal {
    /// Transporte para o canal dado, lendo a conversa (o destino) no armazenamento.
    pub fn novo(store: Arc<Store>, canal: Arc<dyn Integration>) -> Self {
        Self { store, canal }
    }

    /// Canal por trás do transporte (para estado, saúde e contadores).
    pub fn canal(&self) -> &Arc<dyn Integration> {
        &self.canal
    }
}

impl TransporteSaida for EnvioPeloCanal {
    fn enviar(&self, item: ItemSaida) -> FuturoEnvio {
        let store = Arc::clone(&self.store);
        let canal = Arc::clone(&self.canal);
        Box::pin(async move {
            let peer = peer_da_conversa(&store, &item)?;
            let idempotencia = chave_idempotencia(&item);
            let comando = comando_do_item(&item, &peer, &idempotencia)?;
            let resultado = match &comando {
                ComandoSaida::EnviarTexto(c) => canal.enviar_texto(c.clone()).await,
                ComandoSaida::EnviarArquivo(c) => canal.enviar_arquivo(c.clone()).await,
                ComandoSaida::DefinirPresenca(c) => canal.definir_presenca(c.clone()).await,
            };
            resultado.map_err(|erro| falha(&erro))
        })
    }
}

/// Chave de idempotência da saída: a mesma do protocolo e a mesma da `outbox`.
pub fn chave_idempotencia(item: &ItemSaida) -> String {
    format!("{}:{}", item.turn_id, item.seq)
}

/// Destino do item: o identificador do cliente no canal, gravado na conversa.
fn peer_da_conversa(store: &Store, item: &ItemSaida) -> std::result::Result<String, FalhaEnvio> {
    let conversa = store
        .conversas()
        .obter(item.org_id, item.conversation_id)
        .map_err(|erro| FalhaEnvio::definitiva(format!("não consegui ler a conversa: {erro}")))?;
    conversa.peer_externo.clone().ok_or_else(|| {
        FalhaEnvio::definitiva(format!(
            "a conversa `{}` não tem identificador do cliente no canal (peer)",
            item.conversation_id
        ))
    })
}

/// Traduz um item da `outbox` no comando do protocolo.
///
/// O payload é o que as ferramentas do turno gravaram: `text` = `{"texto": "…"}` (HR-13), `file` =
/// `{"blob_ref", "arquivo_id", "nome", "mime", "tamanho_bytes", "sha256"}` (HR-24), `presence` =
/// `{"estado": "digitando"|"pausado"}`.
pub fn comando_do_item(
    item: &ItemSaida,
    peer: &str,
    idempotencia: &str,
) -> std::result::Result<ComandoSaida, FalhaEnvio> {
    let valor: Value = serde_json::from_str(&item.conteudo_json).map_err(|erro| {
        FalhaEnvio::definitiva(format!("payload do item de saída inválido: {erro}"))
    })?;
    match item.tipo.as_str() {
        "text" => {
            let texto = valor
                .get("texto")
                .and_then(Value::as_str)
                .ok_or_else(|| FalhaEnvio::definitiva("item de saída `text` sem `texto`"))?;
            Ok(ComandoSaida::EnviarTexto(SendText {
                idempotencia: idempotencia.to_owned(),
                peer: peer.to_owned(),
                texto: texto.to_owned(),
            }))
        }
        "file" => {
            let campo = |nome: &str| {
                valor
                    .get(nome)
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        FalhaEnvio::definitiva(format!("item de saída `file` sem `{nome}`"))
                    })
            };
            let tamanho_bytes = valor
                .get("tamanho_bytes")
                .and_then(Value::as_u64)
                .ok_or_else(|| {
                    FalhaEnvio::definitiva("item de saída `file` sem `tamanho_bytes`")
                })?;
            Ok(ComandoSaida::EnviarArquivo(SendFile {
                idempotencia: idempotencia.to_owned(),
                peer: peer.to_owned(),
                nome: campo("nome")?,
                mime: campo("mime")?,
                tamanho_bytes,
                sha256: campo("sha256")?,
                blob_ref: campo("blob_ref")?,
                legenda: valor
                    .get("legenda")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            }))
        }
        "presence" => {
            let estado = match valor.get("estado").and_then(Value::as_str) {
                Some("pausado") => Presenca::Pausado,
                Some("digitando") => Presenca::Digitando,
                outro => {
                    return Err(FalhaEnvio::definitiva(format!(
                        "item de saída `presence` com estado desconhecido: {outro:?}"
                    )));
                }
            };
            Ok(ComandoSaida::DefinirPresenca(SetPresence {
                peer: peer.to_owned(),
                estado,
            }))
        }
        outro => Err(FalhaEnvio::definitiva(format!(
            "item de saída com tipo desconhecido: `{outro}`"
        ))),
    }
}

/// Classifica o erro do canal: transitório volta para a fila, o resto é definitivo.
fn falha(erro: &ErroIntegracao) -> FalhaEnvio {
    if erro.eh_transitorio() {
        FalhaEnvio::transitoria(erro.to_string())
    } else {
        FalhaEnvio::definitiva(erro.to_string())
    }
}

// ───────────────────────────── Entrada: canal → inbox ─────────────────────────────

/// Quem decide em qual conversa o evento de entrada cai.
///
/// A conversa não vem no evento do canal (o canal conhece o contato, não a conversa do Harness).
/// A implementação pronta é [`ConversaDoCanal`]; quem precisar de outra regra (por exemplo, uma
/// conversa por grupo de WhatsApp) implementa este trait.
pub trait ResolvedorConversa: Send + Sync + 'static {
    /// Conversa do evento — aberta ou recém-criada.
    fn resolver(&self, org: OrgId, evento: &InboundEvent) -> Resultado<ConversationId>;
}

/// Uma conversa por par (canal, contato), com o agente que atende.
///
/// Procura uma conversa da org com o mesmo canal e o mesmo `peer_externo`; se não houver, abre uma
/// nova com o agente dado.
pub struct ConversaDoCanal {
    store: Arc<Store>,
    agente: AgentId,
    canal: String,
    integracao: Option<IntegrationId>,
    /// Conversa de cada contato já vista, para a mensagem seguinte não varrer o banco de novo.
    cache: Mutex<HashMap<String, ConversationId>>,
}

impl ConversaDoCanal {
    /// Resolvedor para o canal dado. `integracao` é gravada na conversa nova quando existir a
    /// linha de `integrations` (a HR-26 passa o id; sem repositório de integrações ainda, `None`).
    pub fn novo(
        store: Arc<Store>,
        agente: AgentId,
        canal: impl Into<String>,
        integracao: Option<IntegrationId>,
    ) -> Self {
        Self {
            store,
            agente,
            canal: canal.into(),
            integracao,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Quantos contatos estão no cache (para os testes e para quem quiser acompanhar).
    pub fn em_cache(&self) -> usize {
        self.cache
            .lock()
            .unwrap_or_else(|veneno| veneno.into_inner())
            .len()
    }

    /// Conversa lembrada do contato, **conferida** no banco: cache velho não vale.
    ///
    /// A conferência é uma leitura por id (barata). Só vale a pena lembrar conversa aberta: se ela
    /// foi transferida ou fechada, a busca volta ao caminho completo, que prefere uma aberta.
    fn do_cache(&self, org: OrgId, peer: &str) -> Option<ConversationId> {
        let lembrada = {
            let cache = self
                .cache
                .lock()
                .unwrap_or_else(|veneno| veneno.into_inner());
            cache.get(peer).copied()
        }?;
        let conversa = self.store.conversas().obter(org, lembrada).ok();
        let vale = conversa.is_some_and(|conversa| {
            conversa.canal == self.canal
                && conversa.peer_externo.as_deref() == Some(peer)
                && conversa.status == StatusConversa::Aberta
        });
        if vale {
            return Some(lembrada);
        }
        if let Ok(mut cache) = self.cache.lock() {
            cache.remove(peer);
        }
        None
    }

    /// Guarda a conversa do contato no cache (limpando tudo quando passa do teto).
    fn lembrar(&self, peer: &str, conversa: ConversationId) {
        let Ok(mut cache) = self.cache.lock() else {
            return;
        };
        if cache.len() >= TETO_CACHE {
            cache.clear();
        }
        cache.insert(peer.to_owned(), conversa);
    }
}

impl ResolvedorConversa for ConversaDoCanal {
    fn resolver(&self, org: OrgId, evento: &InboundEvent) -> Resultado<ConversationId> {
        if let Some(conversa) = self.do_cache(org, &evento.peer) {
            return Ok(conversa);
        }
        if let Some(conversa) = self.procurar(org, &evento.peer)? {
            self.lembrar(&evento.peer, conversa.id);
            return Ok(conversa.id);
        }
        let nova = self.store.conversas().criar(
            org,
            NovaConversa {
                agent_id: self.agente,
                integration_id: self.integracao,
                canal: self.canal.clone(),
                peer_externo: Some(evento.peer.clone()),
                departamento_alvo: None,
            },
        )?;
        self.lembrar(&evento.peer, nova.id);
        Ok(nova.id)
    }
}

impl ConversaDoCanal {
    /// Varre as conversas da org procurando o par (canal, peer); prefere a que está aberta.
    ///
    /// O repositório de conversas não tem busca por peer (`listar` pagina por id), então a varredura
    /// é limitada a [`PAGINAS_BUSCA`] páginas — o suficiente para a operação normal e registrado
    /// como limite em `docs/PLUGIN_EXEMPLO.md`. A HR-26 deve expor a busca no repositório.
    fn procurar(&self, org: OrgId, peer: &str) -> Resultado<Option<Conversa>> {
        let mut apos = None;
        let mut encontrada: Option<Conversa> = None;
        for _ in 0..PAGINAS_BUSCA {
            let pagina = Pagina { apos, limite: 200 };
            let conversas = self.store.conversas().listar(org, None, pagina)?;
            if conversas.is_empty() {
                break;
            }
            apos = conversas.last().map(|conversa| conversa.id.as_uuid());
            for conversa in conversas {
                if conversa.canal != self.canal || conversa.peer_externo.as_deref() != Some(peer) {
                    continue;
                }
                if conversa.status == StatusConversa::Aberta {
                    return Ok(Some(conversa));
                }
                encontrada.get_or_insert(conversa);
            }
            if apos.is_none() {
                break;
            }
        }
        Ok(encontrada)
    }
}

/// Contadores da entrada, por integração (nunca somados entre orgs).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RelatoEntrada {
    /// Eventos aceitos na fila interna.
    pub recebidos: u64,
    /// Eventos gravados na `inbox` como novos.
    pub gravados: u64,
    /// Eventos que a `inbox` recusou por já existir (id externo repetido).
    pub duplicados: u64,
    /// Eventos que não couberam na fila interna (contrapressão).
    pub recusados: u64,
    /// Eventos que falharam ao gravar (banco, conversa).
    pub falhas: u64,
    /// Eventos de estado do canal (conectado, QR, erro).
    pub estados: u64,
    /// Eventos com anexo (o anexo ainda não vira linha em `attachments`; ver limites no doc).
    pub com_anexo: u64,
}

#[derive(Debug, Default)]
struct Contadores {
    recebidos: AtomicU64,
    gravados: AtomicU64,
    duplicados: AtomicU64,
    recusados: AtomicU64,
    falhas: AtomicU64,
    estados: AtomicU64,
    com_anexo: AtomicU64,
}

impl Contadores {
    fn relato(&self) -> RelatoEntrada {
        RelatoEntrada {
            recebidos: self.recebidos.load(Ordering::Relaxed),
            gravados: self.gravados.load(Ordering::Relaxed),
            duplicados: self.duplicados.load(Ordering::Relaxed),
            recusados: self.recusados.load(Ordering::Relaxed),
            falhas: self.falhas.load(Ordering::Relaxed),
            estados: self.estados.load(Ordering::Relaxed),
            com_anexo: self.com_anexo.load(Ordering::Relaxed),
        }
    }
}

/// Receptor dos eventos do canal: grava o que chega na `inbox` da HR-12.
///
/// O trait [`Receptor`] entrega o evento de forma síncrona, no laço de leitura do canal; por isso
/// [`receber`](Receptor::receber) só **enfileira** e uma tarefa própria faz a gravação — quem
/// segura o canal é o banco, nunca o plugin.
pub struct InboxDoCanal {
    fluxo: Fluxo,
    org: OrgId,
    resolvedor: Arc<dyn ResolvedorConversa>,
    sender: mpsc::Sender<InboundEvent>,
    contadores: Arc<Contadores>,
    ultimo_estado: Mutex<Option<EventoEstado>>,
}

impl InboxDoCanal {
    /// Cria o receptor e sobe a tarefa que grava na `inbox`.
    ///
    /// A tarefa vive enquanto este `Arc` (ou o [`Fluxo`]) existir: quando todos os handles caem, o
    /// canal fecha e a tarefa termina.
    pub fn iniciar(fluxo: Fluxo, org: OrgId, resolvedor: Arc<dyn ResolvedorConversa>) -> Arc<Self> {
        let (sender, mut receiver) = mpsc::channel(FILA_EVENTOS);
        let contadores = Arc::new(Contadores::default());
        let receptor = Arc::new(Self {
            fluxo,
            org,
            resolvedor,
            sender,
            contadores: Arc::clone(&contadores),
            ultimo_estado: Mutex::new(None),
        });
        let gravador = Arc::clone(&receptor);
        tokio::spawn(async move {
            while let Some(evento) = receiver.recv().await {
                gravador.gravar(evento);
            }
        });
        receptor
    }

    /// Contadores da entrada.
    pub fn contadores(&self) -> RelatoEntrada {
        self.contadores.relato()
    }

    /// Último estado que o canal reportou (conectado, QR, erro).
    pub fn ultimo_estado(&self) -> Option<EventoEstado> {
        self.ultimo_estado
            .lock()
            .unwrap_or_else(|veneno| veneno.into_inner())
            .clone()
    }

    /// Grava o evento na `inbox` (idempotente por integração + id externo).
    ///
    /// Devolve a situação para quem quiser chamar direto (testes, reentrega manual).
    pub fn gravar(&self, evento: InboundEvent) -> Option<Recebido> {
        let conversa = match self.resolvedor.resolver(self.org, &evento) {
            Ok(conversa) => conversa,
            Err(erro) => {
                self.contadores.falhas.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(
                    org = %self.org,
                    integracao = %evento.integracao,
                    id_externo = %evento.id_externo,
                    %erro,
                    "não consegui resolver a conversa do evento de entrada"
                );
                return None;
            }
        };
        if !evento.anexos.is_empty() {
            self.contadores.com_anexo.fetch_add(1, Ordering::Relaxed);
        }
        let recebida = MensagemRecebida {
            integration_id: evento.integracao,
            id_externo: evento.id_externo.clone(),
            conversa,
            texto: evento.texto.clone(),
            anexos_json: anexos_json(&evento),
        };
        match self.fluxo.receber_mensagem(self.org, recebida) {
            Ok(gravado) => match gravado.situacao {
                Recebido::Novo => {
                    self.contadores.gravados.fetch_add(1, Ordering::Relaxed);
                    Some(Recebido::Novo)
                }
                Recebido::Duplicado => {
                    self.contadores.duplicados.fetch_add(1, Ordering::Relaxed);
                    Some(Recebido::Duplicado)
                }
            },
            Err(erro) => {
                self.contadores.falhas.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(
                    org = %self.org,
                    integracao = %evento.integracao,
                    id_externo = %evento.id_externo,
                    %erro,
                    "não consegui gravar o evento de entrada na inbox"
                );
                None
            }
        }
    }
}

impl Receptor for InboxDoCanal {
    fn receber(&self, evento: EventoIntegracao) {
        match evento {
            EventoIntegracao::Entrada(entrada) => match self.sender.try_send(entrada) {
                Ok(()) => {
                    self.contadores.recebidos.fetch_add(1, Ordering::Relaxed);
                }
                Err(erro) => {
                    self.contadores.recusados.fetch_add(1, Ordering::Relaxed);
                    tracing::warn!(%erro, "fila de entrada cheia: evento do canal recusado");
                }
            },
            EventoIntegracao::Estado(estado) => {
                self.contadores.estados.fetch_add(1, Ordering::Relaxed);
                *self
                    .ultimo_estado
                    .lock()
                    .unwrap_or_else(|veneno| veneno.into_inner()) = Some(estado);
            }
        }
    }
}

/// Anexos do evento, como o canal entregou.
///
/// Enquanto a HR-26 não registrar a linha em `attachments` (com `blob_path` e extração), o anexo
/// fica descrito no próprio item da mensagem — nada se perde, e o campo passa a receber ids quando
/// o registro existir.
fn anexos_json(evento: &InboundEvent) -> String {
    if evento.anexos.is_empty() {
        return "[]".to_owned();
    }
    serde_json::to_string(&evento.anexos).unwrap_or_else(|_| "[]".to_owned())
}

#[cfg(test)]
mod testes {
    use super::*;
    use poph_store::EstadoSaida;
    use poph_types::{OutboxId, TurnId};

    fn item(tipo: &str, conteudo: Value) -> ItemSaida {
        ItemSaida {
            id: OutboxId::generate(),
            org_id: OrgId::generate(),
            conversation_id: ConversationId::generate(),
            turn_id: TurnId::generate(),
            seq: 3,
            tipo: tipo.to_owned(),
            conteudo_json: conteudo.to_string(),
            estado: EstadoSaida::Pendente,
            tentativas: 0,
            ultimo_erro: None,
            criado_em: 0,
            enviado_em: None,
        }
    }

    #[test]
    fn texto_vira_enviar_texto_com_a_chave_da_outbox() {
        let item = item("text", serde_json::json!({"texto": "olá"}));
        let comando = comando_do_item(&item, "5511", &chave_idempotencia(&item)).unwrap();
        let ComandoSaida::EnviarTexto(texto) = comando else {
            panic!("esperava enviar_texto");
        };
        assert_eq!(texto.peer, "5511");
        assert_eq!(texto.texto, "olá");
        assert_eq!(texto.idempotencia, format!("{}:3", item.turn_id));
    }

    #[test]
    fn arquivo_vira_enviar_arquivo_com_o_blob() {
        let item = item(
            "file",
            serde_json::json!({
                "blob_ref": "org/sha",
                "arquivo_id": "01a1f6a5-cd9b-7000-8000-000000000000",
                "nome": "manual.pdf",
                "mime": "application/pdf",
                "tamanho_bytes": 1234,
                "sha256": "a".repeat(64),
                "legenda": "segue"
            }),
        );
        let comando = comando_do_item(&item, "5511", "t:1").unwrap();
        let ComandoSaida::EnviarArquivo(arquivo) = comando else {
            panic!("esperava enviar_arquivo");
        };
        assert_eq!(arquivo.nome, "manual.pdf");
        assert_eq!(arquivo.blob_ref, "org/sha");
        assert_eq!(arquivo.tamanho_bytes, 1234);
        assert_eq!(arquivo.legenda.as_deref(), Some("segue"));
    }

    #[test]
    fn presenca_e_tipo_desconhecido() {
        let comando = comando_do_item(
            &item("presence", serde_json::json!({"estado": "digitando"})),
            "5511",
            "t:2",
        )
        .unwrap();
        assert!(matches!(comando, ComandoSaida::DefinirPresenca(_)));

        let erro =
            comando_do_item(&item("texto", serde_json::json!({})), "5511", "t:2").unwrap_err();
        assert!(
            !erro.transitoria,
            "tipo desconhecido não deve ser retentado"
        );
    }

    #[test]
    fn texto_sem_campo_e_falha_definitiva() {
        let erro = comando_do_item(
            &item("text", serde_json::json!({"outro": 1})),
            "5511",
            "t:1",
        )
        .unwrap_err();
        assert!(!erro.transitoria);
        assert!(erro.motivo.contains("`texto`"), "{}", erro.motivo);
    }

    #[test]
    fn anexo_do_evento_fica_descrito_na_mensagem() {
        let evento = InboundEvent {
            integracao: IntegrationId::generate(),
            id_externo: "abc".to_owned(),
            peer: "5511".to_owned(),
            remetente: None,
            texto: "segue".to_owned(),
            anexos: vec![poph_types::AnexoEntrada::Blob {
                nome: "manual.pdf".to_owned(),
                mime: "application/pdf".to_owned(),
                sha256: "a".repeat(64),
                tamanho_bytes: 10,
            }],
            ocorrido_em: 1,
            metadados: Value::Null,
        };
        let json = anexos_json(&evento);
        assert!(json.contains("manual.pdf"), "{json}");

        let sem = InboundEvent {
            anexos: Vec::new(),
            ..evento
        };
        assert_eq!(anexos_json(&sem), "[]");
    }
}
