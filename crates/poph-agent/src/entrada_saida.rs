//! Entrada e saída: idempotência, fila durável, retomada na subida e backoff (HR-12).
//!
//! O que este módulo garante:
//! - **Uma mensagem entregue duas vezes é processada uma vez.** A `inbox` tem chave única
//!   `(integration_id, external_message_id)`; a segunda entrega devolve [`Recebido::Duplicado`] sem
//!   erro e sem gerar segundo turno. O sucesso só é devolvido ao canal **depois** do commit: quem
//!   chama [`Fluxo::receber`] pode responder OK ao webhook assim que ela retorna.
//! - **A entrada é durável até o turno concluir.** O item da `inbox` só ganha `processed_at` quando
//!   o turno chama [`Fluxo::concluir_entrada`]. Queda no meio deixa o item pendente e a retomada o
//!   despacha de novo (pelo menos uma vez).
//! - **Fila cheia não perde evento.** Quando o ator da conversa recusa o evento, o item continua
//!   pendente e é retentado com backoff exponencial.
//! - **A saída é idempotente por `(turn_id, seq)`** e passa por `pending → sending → sent | failed`.
//!   A transição para `sending` é gravada **antes** do efeito externo; a retomada reenvia o que
//!   ficou em `sending` (D11: entrega pelo menos uma vez, risco de duplicata aceito).
//!
//! O que ele **não** faz: o turno com LLM, RAG, Jev e ferramentas (HR-13); o transporte real de
//! cada canal (HR-25/HR-26). Quem liga o transporte é [`TransporteSaida`].

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use poph_store::{
    EstadoSaida, ItemSaida, NovaEntrada, NovaEntradaComMensagem, Pagina, Recebido,
    RecebidoComMensagem, Store,
};
use poph_types::{ConversationId, InboxId, IntegrationId, MessageId, OrgId, OutboxId};

use crate::atores::{Atores, Tarefa};
use crate::erro::{ErroAgente, Resultado};
use crate::metricas::MetricasFluxo;

// ───────────────────────────── Evento normalizado ─────────────────────────────

/// O evento que a `inbox` carrega: qual conversa e qual mensagem o turno deve processar.
///
/// É o mínimo que o despacho precisa; os dados do canal (texto, anexos, peer) já estão em
/// `messages`/`attachments`. Campos desconhecidos no JSON são ignorados, para o formato poder
/// crescer nas integrações (HR-25/HR-26) sem quebrar item antigo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EventoEntrada {
    /// Conversa que recebeu a mensagem.
    pub conversa: ConversationId,
    /// Mensagem gravada em `messages`.
    pub mensagem: MessageId,
}

impl EventoEntrada {
    /// Serializa para o `payload_json` da `inbox`.
    pub fn para_json(self) -> Resultado<String> {
        serde_json::to_string(&self).map_err(|erro| {
            ErroAgente::Interno(format!(
                "não consegui serializar o evento de entrada: {erro}"
            ))
        })
    }

    /// Lê o `payload_json` de um item da `inbox`.
    pub fn de_json(payload: &str) -> Resultado<Self> {
        serde_json::from_str(payload).map_err(|erro| ErroAgente::EventoInvalido {
            motivo: erro.to_string(),
        })
    }
}

/// Mensagem recebida de uma integração, pronta para gravar de forma atômica.
#[derive(Debug, Clone)]
pub struct MensagemRecebida {
    /// Integração de origem.
    pub integration_id: IntegrationId,
    /// Id da mensagem no canal (chave de idempotência da entrega).
    pub id_externo: String,
    /// Conversa que recebe.
    pub conversa: ConversationId,
    /// Texto recebido (vazio é permitido).
    pub texto: String,
    /// Ids de anexos em JSON (vazio = `[]`).
    pub anexos_json: String,
}

// ───────────────────────────── Envio (transporte) ─────────────────────────────

/// Futuro do envio externo, sem depender de `async_trait`.
pub type FuturoEnvio = Pin<Box<dyn Future<Output = Result<(), FalhaEnvio>> + Send + 'static>>;

/// Itens lidos da caixa de saída ao filtrar só o console (HR-26f): o máximo que o banco devolve.
const LOTE_MAXIMO_DO_CONSOLE: u32 = 1000;

/// Quem entrega a mensagem ao canal: Evolution, plugin, console.
///
/// O fluxo só chama `enviar` **depois** de gravar `sending`; devolver `Ok` faz o item virar `sent`,
/// e `Err` decide entre retentar (transitória) e desistir (definitiva). Reenviar o mesmo item pode
/// duplicar a mensagem no canal — é o preço da regra D11.
pub trait TransporteSaida: Send + Sync + 'static {
    /// Entrega o item ao canal.
    fn enviar(&self, item: ItemSaida) -> FuturoEnvio;
}

/// Falha de envio, com o que decide se vale retentar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FalhaEnvio {
    /// Motivo legível (vai para `outbox.last_error` e para o relatório).
    pub motivo: String,
    /// `true` = rede/canal ocupado: retenta com backoff. `false` = erro do conteúdo/destino.
    pub transitoria: bool,
}

impl FalhaEnvio {
    /// Falha que vale retentar (rede, 429, 5xx, timeout).
    pub fn transitoria(motivo: impl Into<String>) -> Self {
        Self {
            motivo: motivo.into(),
            transitoria: true,
        }
    }

    /// Falha que não adianta retentar (destinatário inválido, conteúdo recusado).
    pub fn definitiva(motivo: impl Into<String>) -> Self {
        Self {
            motivo: motivo.into(),
            transitoria: false,
        }
    }
}

// ───────────────────────────── Backoff ─────────────────────────────

/// Espera exponencial com teto: `base`, `base × fator`, `base × fator²`… sem passar do teto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    /// Espera da primeira repetição.
    pub base: Duration,
    /// Multiplicador de cada repetição (≥ 1).
    pub fator: u32,
    /// Teto da espera.
    pub teto: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            base: Duration::from_millis(500),
            fator: 2,
            teto: Duration::from_secs(60),
        }
    }
}

impl Backoff {
    /// Espera depois da `tentativa`-ésima falha (1 = primeira).
    pub fn espera(&self, tentativa: u32) -> Duration {
        let mut espera = self.base;
        for _ in 1..tentativa.max(1) {
            if espera >= self.teto {
                break;
            }
            espera = espera.saturating_mul(self.fator.max(1));
        }
        espera.min(self.teto)
    }
}

// ───────────────────────────── Ajustes e relatos ─────────────────────────────

/// Ajustes do fluxo.
#[derive(Debug, Clone)]
pub struct OpcoesFluxo {
    /// Itens de entrada lidos por ciclo e por org.
    pub lote_entrada: u32,
    /// Itens de saída lidos por ciclo e por org.
    pub lote_saida: u32,
    /// Tentativas antes de desistir (entrada: registrar sem processar; saída: `failed`).
    pub max_tentativas: u32,
    /// Espera entre tentativas.
    pub backoff: Backoff,
    /// Intervalo do laço de fundo ([`Fluxo::iniciar_laco`]).
    pub intervalo_ciclo: Duration,
    /// Tentativas de um turno quando o modelo falha, contando a primeira (HR-26i). Esgotadas, o item
    /// é concluído sem resposta.
    pub tentativas_falha_modelo: u32,
    /// Espera antes de cada nova tentativa de um turno com falha do modelo (HR-26i). A primeira
    /// espera é `base`, e cada uma seguinte é multiplicada pelo `fator`, até o `teto`.
    pub atraso_falha_modelo: Backoff,
}

impl OpcoesFluxo {
    /// Ajusta a política de falha do modelo (HR-26i): `tentativas` no total e `atraso_base` antes da
    /// segunda tentativa; cada espera seguinte é quatro vezes a anterior, até 10 minutos.
    pub fn com_falha_modelo(mut self, tentativas: u32, atraso_base: Duration) -> Self {
        self.tentativas_falha_modelo = tentativas;
        self.atraso_falha_modelo = Backoff {
            base: atraso_base,
            fator: 4,
            teto: Duration::from_secs(600),
        };
        self
    }
}

impl Default for OpcoesFluxo {
    fn default() -> Self {
        Self {
            lote_entrada: 200,
            lote_saida: 100,
            max_tentativas: 8,
            backoff: Backoff::default(),
            intervalo_ciclo: Duration::from_millis(250),
            tentativas_falha_modelo: 3,
            atraso_falha_modelo: Backoff {
                base: Duration::from_secs(30),
                fator: 4,
                teto: Duration::from_secs(600),
            },
        }
    }
}

/// O que aconteceu num despacho de entrada.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RelatoDespacho {
    /// Eventos entregues ao ator da conversa.
    pub despachados: u32,
    /// Eventos adiados (backoff ou já em voo).
    pub adiados: u32,
    /// Eventos cujo payload não pôde ser lido (ficaram registrados com erro).
    pub invalidos: u32,
    /// Eventos que esgotaram as tentativas de despacho (ficaram registrados com erro).
    pub desistentes: u32,
}

impl RelatoDespacho {
    /// Soma outro relato (usada ao percorrer as orgs).
    pub fn somar(&mut self, outro: Self) {
        self.despachados += outro.despachados;
        self.adiados += outro.adiados;
        self.invalidos += outro.invalidos;
        self.desistentes += outro.desistentes;
    }
}

/// O que aconteceu num envio de saída.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RelatoEnvio {
    /// Itens confirmados no canal (`sent`).
    pub enviados: u32,
    /// Itens adiados (ainda no backoff).
    pub adiados: u32,
    /// Itens que falharam nesta passagem (transitória ou definitiva).
    pub falharam: u32,
    /// Itens que esgotaram as tentativas e foram para `failed`.
    pub esgotados: u32,
}

impl RelatoEnvio {
    /// Soma outro relato.
    pub fn somar(&mut self, outro: Self) {
        self.enviados += outro.enviados;
        self.adiados += outro.adiados;
        self.falharam += outro.falharam;
        self.esgotados += outro.esgotados;
    }
}

/// O que a retomada da subida fez.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RelatoRetomada {
    /// Entrada despachada de novo.
    pub entrada: RelatoDespacho,
    /// Saída processada (inclui o reenvio).
    pub saida: RelatoEnvio,
    /// Itens que estavam em `sending` quando a retomada começou.
    pub reenvios: u32,
}

/// Tamanho das filas de uma org.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Estatisticas {
    /// Itens da `inbox` ainda não processados.
    pub entrada_pendente: u64,
    /// Itens da `outbox` em `sending`.
    pub saida_enviando: u64,
    /// Itens da `outbox` ainda em `pending`.
    pub saida_pendente: u64,
    /// Itens da `outbox` em `failed`.
    pub saida_falhou: u64,
}

// ───────────────────────────── Estado interno ─────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DesfechoEnvio {
    Adiado,
    Enviado,
    Falhou,
    Esgotado,
}

#[derive(Debug, Clone, Copy)]
struct Agenda {
    proximo_em: Instant,
    tentativa: u32,
}

type Relogio = Arc<dyn Fn() -> Instant + Send + Sync>;

struct Interno {
    store: Arc<Store>,
    atores: Atores,
    opcoes: OpcoesFluxo,
    metricas: MetricasFluxo,
    relogio: Mutex<Relogio>,
    /// Itens entregues ao ator e ainda não concluídos pelo turno.
    em_voo: Mutex<HashSet<InboxId>>,
    /// Itens de saída em processo de envio (dois ciclos concorrentes não enviam o mesmo item).
    em_envio: Mutex<HashSet<OutboxId>>,
    /// Itens com o despacho adiado por backoff.
    agenda_entrada: Mutex<HashMap<InboxId, Agenda>>,
    /// Itens com o envio adiado por backoff.
    agenda_saida: Mutex<HashMap<OutboxId, Agenda>>,
}

/// Fluxo de entrada e saída de uma instância. É `Clone` barato (compartilha o mesmo estado).
#[derive(Clone)]
pub struct Fluxo {
    interno: Arc<Interno>,
}

impl Fluxo {
    /// Cria o fluxo com métricas próprias (não publicadas em `/metrics`).
    pub fn nova(store: Arc<Store>, atores: Atores, opcoes: OpcoesFluxo) -> Self {
        Self::com_metricas(store, atores, opcoes, MetricasFluxo::proprio())
    }

    /// Cria o fluxo publicando nas métricas dadas (as do processo, em produção).
    pub fn com_metricas(
        store: Arc<Store>,
        atores: Atores,
        opcoes: OpcoesFluxo,
        metricas: MetricasFluxo,
    ) -> Self {
        Self {
            interno: Arc::new(Interno {
                store,
                atores,
                opcoes,
                metricas,
                relogio: Mutex::new(Arc::new(Instant::now)),
                em_voo: Mutex::new(HashSet::new()),
                em_envio: Mutex::new(HashSet::new()),
                agenda_entrada: Mutex::new(HashMap::new()),
                agenda_saida: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Troca o relógio usado nas esperas (testes avançam o tempo sem dormir).
    ///
    /// Vale para todas as cópias deste fluxo, porque o relógio mora no estado compartilhado.
    pub fn com_relogio(&self, relogio: Relogio) {
        *self
            .interno
            .relogio
            .lock()
            .unwrap_or_else(|erro| erro.into_inner()) = relogio;
    }

    /// Métricas do fluxo.
    pub fn metricas(&self) -> &MetricasFluxo {
        &self.interno.metricas
    }

    /// Store por trás do fluxo (a HR-13 grava mensagens e turnos com ele).
    pub fn store(&self) -> &Arc<Store> {
        &self.interno.store
    }

    // ───────────────────────── Entrada ─────────────────────────

    /// Grava o evento na `inbox`. Só retorna depois do commit: quem chama pode dar o ACK do canal
    /// em seguida. Reentrega devolve [`Recebido::Duplicado`] sem erro.
    pub fn receber(&self, org: OrgId, nova: NovaEntrada) -> Resultado<Recebido> {
        let (situacao, _item) = self.interno.store.caixa_entrada().receber(org, nova)?;
        match situacao {
            Recebido::Novo => self.interno.metricas.entradas_novas.increment(1),
            Recebido::Duplicado => self.interno.metricas.entradas_duplicadas.increment(1),
        }
        Ok(situacao)
    }

    /// Grava a mensagem do cliente **e** o item da `inbox` numa única transação, e devolve os dois.
    ///
    /// É o caminho da integração: ou as duas linhas existem, ou nenhuma. A mensagem é gravada em
    /// `messages` (direção `in`, papel `user`) e o evento da `inbox` já carrega os ids que o turno
    /// precisa.
    pub fn receber_mensagem(
        &self,
        org: OrgId,
        recebida: MensagemRecebida,
    ) -> Resultado<RecebidoComMensagem> {
        let mensagem = MessageId::generate();
        let evento = EventoEntrada {
            conversa: recebida.conversa,
            mensagem,
        }
        .para_json()?;
        let gravado = self.interno.store.caixa_entrada().receber_com_mensagem(
            org,
            NovaEntradaComMensagem {
                integration_id: recebida.integration_id,
                id_externo: recebida.id_externo,
                evento_json: evento,
                conversation_id: recebida.conversa,
                mensagem_id: mensagem,
                texto: recebida.texto,
                anexos_json: recebida.anexos_json,
            },
        )?;
        match gravado.situacao {
            Recebido::Novo => self.interno.metricas.entradas_novas.increment(1),
            Recebido::Duplicado => self.interno.metricas.entradas_duplicadas.increment(1),
        }
        Ok(gravado)
    }

    /// Fecha o item da `inbox`: o turno terminou (com ou sem erro) e ele sai da fila.
    ///
    /// Deve ser chamada pelo processador de turno (HR-13) com `tarefa.entrada`. Sem ela, o item
    /// continua pendente e volta a ser despachado na próxima retomada.
    pub fn concluir_entrada(&self, org: OrgId, id: InboxId, erro: Option<&str>) -> Resultado<()> {
        self.interno
            .store
            .caixa_entrada()
            .marcar_processado(org, id, erro)?;
        self.interno.em_voo().remove(&id);
        self.interno.agenda_entrada().remove(&id);
        self.interno.metricas.entradas_concluidas.increment(1);
        Ok(())
    }

    /// Ajustes em uso (o processador de turno lê a política de falha do modelo daqui).
    pub fn opcoes(&self) -> &OpcoesFluxo {
        &self.interno.opcoes
    }

    /// Adia itens da `inbox` por falha do modelo (HR-26i): continuam pendentes, sem processar, e só
    /// voltam a ser despachados depois de `proxima_em` (ms desde a época, no relógio do banco). Não
    /// mexe na saída: nada vai para a `outbox` por causa do adiamento.
    pub fn adiar_entradas(
        &self,
        org: OrgId,
        ids: &[InboxId],
        proxima_em: i64,
        erro: &str,
    ) -> Resultado<()> {
        self.interno
            .store
            .caixa_entrada()
            .adiar(org, ids, proxima_em, erro)?;
        for id in ids {
            self.interno.liberar_entrada(*id);
        }
        Ok(())
    }

    /// Solta a reserva em memória de um item que o turno **não** conseguiu atender por motivo
    /// transitório (sem vaga, cota da org), sem marcar como processado.
    ///
    /// Sem isso o item ficaria reservado para sempre e nunca voltaria a ser despachado. Com isso ele
    /// volta no próximo ciclo — a espera pela vaga dentro do turno é o que impede um laço apertado.
    pub fn devolver_entrada(&self, id: InboxId) {
        self.interno.liberar_entrada(id);
    }

    /// Despacha os itens pendentes de uma org para o ator da conversa de cada um.
    ///
    /// Item recusado pelo ator (fila cheia) continua pendente e volta no próximo ciclo dentro do
    /// backoff. Item já em voo não é despachado de novo.
    pub fn despachar(&self, org: OrgId) -> Resultado<RelatoDespacho> {
        let itens = self
            .interno
            .store
            .caixa_entrada()
            .listar_pendentes(org, self.interno.opcoes.lote_entrada)?;
        self.despachar_itens(org, itens)
    }

    /// Despacha uma lista de itens já lida da `inbox`. Mesma regra de [`Fluxo::despachar`].
    pub fn despachar_itens(
        &self,
        org: OrgId,
        itens: Vec<poph_store::ItemEntrada>,
    ) -> Resultado<RelatoDespacho> {
        let agora = self.interno.agora();
        let mut relato = RelatoDespacho::default();
        for item in itens {
            let id = item.id;
            // Reservar antes de fazer qualquer coisa: dois ciclos concorrentes não despacham o
            // mesmo item. A reserva só cai quando o turno conclui (`concluir_entrada`).
            if !self.interno.reivindicar_entrada(id) {
                relato.adiados += 1;
                continue;
            }
            // A lista pode ter sido lida antes de o turno concluir este item. A conclusão grava o
            // `processado_em` antes de soltar a reserva, então com a reserva na mão o banco já diz a
            // verdade: se o item está processado, não é despachado de novo.
            match self.interno.store.caixa_entrada().obter(org, id) {
                Ok(atual) if atual.processado_em.is_some() => {
                    self.interno.agenda_entrada().remove(&id);
                    self.interno.liberar_entrada(id);
                    continue;
                }
                Ok(_) => {}
                Err(erro) => {
                    self.interno.liberar_entrada(id);
                    return Err(erro.into());
                }
            }
            if let Some(agenda) = self.interno.agenda_entrada().get(&id)
                && agora < agenda.proximo_em
            {
                self.interno.liberar_entrada(id);
                relato.adiados += 1;
                continue;
            }

            let evento = match EventoEntrada::de_json(&item.evento_json) {
                Ok(evento) => evento,
                Err(erro) => {
                    let marcado = self.interno.store.caixa_entrada().marcar_processado(
                        org,
                        id,
                        Some(&format!("evento de entrada inválido: {erro}")),
                    );
                    self.interno.agenda_entrada().remove(&id);
                    self.interno.liberar_entrada(id);
                    marcado?;
                    self.interno.metricas.entradas_invalidas.increment(1);
                    relato.invalidos += 1;
                    continue;
                }
            };

            let tarefa = Tarefa::da_entrada(org, evento.conversa, evento.mensagem, id);
            match self.interno.atores.enviar(tarefa) {
                Ok(()) => {
                    // A reserva fica: o item está em voo até o turno concluir.
                    self.interno.agenda_entrada().remove(&id);
                    relato.despachados += 1;
                }
                Err(erro) => {
                    let tentativa = self
                        .interno
                        .agenda_entrada()
                        .get(&id)
                        .map_or(1, |agenda| agenda.tentativa.saturating_add(1));
                    self.interno.liberar_entrada(id);
                    if tentativa >= self.interno.opcoes.max_tentativas {
                        // Não some: fica gravado na inbox com o motivo, para a operação ver.
                        let marcado = self.interno.store.caixa_entrada().marcar_processado(
                            org,
                            id,
                            Some(&format!("despacho desistiu: {erro}")),
                        );
                        self.interno.agenda_entrada().remove(&id);
                        marcado?;
                        self.interno.metricas.entradas_desistentes.increment(1);
                        relato.desistentes += 1;
                    } else {
                        let proximo_em = agora + self.interno.opcoes.backoff.espera(tentativa);
                        self.interno.agenda_entrada().insert(
                            id,
                            Agenda {
                                proximo_em,
                                tentativa,
                            },
                        );
                        self.interno.metricas.entradas_adiadas.increment(1);
                        relato.adiados += 1;
                    }
                }
            }
        }
        Ok(relato)
    }

    /// Despacha todas as orgs.
    pub fn despachar_todas(&self) -> Resultado<RelatoDespacho> {
        let mut total = RelatoDespacho::default();
        for org in self.orgs()? {
            total.somar(self.despachar(org)?);
        }
        Ok(total)
    }

    // ───────────────────────── Saída ─────────────────────────

    /// Enfileira a saída. Devolve `false` quando `(turn_id, seq)` já existia (nada foi duplicado).
    pub fn enfileirar_saida(
        &self,
        org: OrgId,
        nova: poph_store::NovaSaida,
    ) -> Resultado<(bool, ItemSaida)> {
        Ok(self.interno.store.caixa_saida().enfileirar(org, nova)?)
    }

    /// Envia o que está aguardando numa org: `pending` e `sending` (o `sending` inclui o reenvio da
    /// retomada). Grava `sending` antes de chamar o canal, `sent` ao confirmar, `failed` no erro
    /// definitivo ou no esgotamento das tentativas.
    pub async fn enviar_pendentes(
        &self,
        org: OrgId,
        transporte: &dyn TransporteSaida,
    ) -> Resultado<RelatoEnvio> {
        let itens = self
            .interno
            .store
            .caixa_saida()
            .listar_pendentes(org, self.interno.opcoes.lote_saida)?;
        self.enviar_itens(org, itens, transporte).await
    }

    /// Envia só o que é de conversa do console (sem integração de canal), HR-26f. É o que o
    /// `poph chat` chama: a caixa de saída da org também tem itens de WhatsApp, que este processo
    /// não entrega e não pode confirmar. Itens de outras conversas ficam como estão.
    pub async fn enviar_pendentes_do_console(
        &self,
        org: OrgId,
        transporte: &dyn TransporteSaida,
    ) -> Resultado<RelatoEnvio> {
        // Lê até o máximo do banco: itens de WhatsApp pendentes à frente não podem tapar os do console.
        let pendentes = self
            .interno
            .store
            .caixa_saida()
            .listar_pendentes(org, LOTE_MAXIMO_DO_CONSOLE)?;
        let mut itens = Vec::new();
        for item in pendentes {
            let conversa = self
                .interno
                .store
                .conversas()
                .obter(org, item.conversation_id)?;
            if conversa.integration_id.is_none() {
                itens.push(item);
            }
        }
        self.enviar_itens(org, itens, transporte).await
    }

    /// Envia uma lista de itens já escolhida, com a mesma regra da fila: reserva, estados e tentativas.
    async fn enviar_itens(
        &self,
        org: OrgId,
        itens: Vec<ItemSaida>,
        transporte: &dyn TransporteSaida,
    ) -> Resultado<RelatoEnvio> {
        let agora = self.interno.agora();
        let mut relato = RelatoEnvio::default();
        for item in itens {
            let id = item.id;
            if !self.interno.reivindicar_saida(id) {
                relato.adiados += 1;
                continue;
            }
            let desfecho = self.enviar_um(org, item, transporte, agora).await;
            self.interno.liberar_saida(id);
            match desfecho? {
                DesfechoEnvio::Adiado => relato.adiados += 1,
                DesfechoEnvio::Enviado => relato.enviados += 1,
                DesfechoEnvio::Falhou => relato.falharam += 1,
                DesfechoEnvio::Esgotado => relato.esgotados += 1,
            }
        }
        Ok(relato)
    }

    /// Tenta enviar um item. Quem chama já reservou o item e solta a reserva depois.
    async fn enviar_um(
        &self,
        org: OrgId,
        item: ItemSaida,
        transporte: &dyn TransporteSaida,
        agora: Instant,
    ) -> Resultado<DesfechoEnvio> {
        let id = item.id;
        if let Some(agenda) = self.interno.agenda_saida().get(&id)
            && agora < agenda.proximo_em
        {
            return Ok(DesfechoEnvio::Adiado);
        }
        if item.tentativas >= self.interno.opcoes.max_tentativas {
            self.interno.store.caixa_saida().falhar_envio(
                org,
                id,
                "tentativas de envio esgotadas",
                true,
            )?;
            self.interno.agenda_saida().remove(&id);
            self.interno.metricas.saidas_esgotadas.increment(1);
            return Ok(DesfechoEnvio::Esgotado);
        }

        // A transição precisa estar no banco antes do efeito externo (D11).
        let tentativa = match self.interno.store.caixa_saida().iniciar_envio(org, id) {
            Ok(tentativa) => tentativa,
            Err(erro) => {
                tracing::warn!(
                    org = %org,
                    saida = %id,
                    erro = %erro,
                    "item de saída não pôde entrar em sending; fica para o próximo ciclo"
                );
                return Ok(DesfechoEnvio::Adiado);
            }
        };

        match transporte.enviar(item).await {
            Ok(()) => {
                self.interno.store.caixa_saida().confirmar_envio(org, id)?;
                self.interno.agenda_saida().remove(&id);
                self.interno.metricas.saidas_enviadas.increment(1);
                Ok(DesfechoEnvio::Enviado)
            }
            Err(falha) if falha.transitoria => {
                self.interno
                    .store
                    .caixa_saida()
                    .falhar_envio(org, id, &falha.motivo, false)?;
                let proximo_em = agora + self.interno.opcoes.backoff.espera(tentativa);
                self.interno.agenda_saida().insert(
                    id,
                    Agenda {
                        proximo_em,
                        tentativa,
                    },
                );
                self.interno.metricas.saidas_transitorias.increment(1);
                Ok(DesfechoEnvio::Falhou)
            }
            Err(falha) => {
                self.interno
                    .store
                    .caixa_saida()
                    .falhar_envio(org, id, &falha.motivo, true)?;
                self.interno.agenda_saida().remove(&id);
                self.interno.metricas.saidas_definitivas.increment(1);
                Ok(DesfechoEnvio::Falhou)
            }
        }
    }

    // ───────────────────────── Retomada e ciclo ─────────────────────────

    /// Retomada da subida: reenvia o que ficou em `sending`, despacha a `inbox` pendente e envia a
    /// saída que sobrou. Conta os reenvios para a métrica (D11).
    pub async fn retomar(&self, transporte: &dyn TransporteSaida) -> Resultado<RelatoRetomada> {
        let orgs = self.orgs()?;
        let mut reenvios = 0u32;
        for org in &orgs {
            reenvios += u32::try_from(
                self.interno
                    .store
                    .caixa_saida()
                    .contar_por_estado(*org, EstadoSaida::Enviando)?,
            )
            .unwrap_or(u32::MAX);
        }
        if reenvios > 0 {
            self.interno
                .metricas
                .reenvios_retomada
                .increment(u64::from(reenvios));
        }

        let mut entrada = RelatoDespacho::default();
        let mut saida = RelatoEnvio::default();
        for org in &orgs {
            entrada.somar(self.despachar(*org)?);
            saida.somar(self.enviar_pendentes(*org, transporte).await?);
        }
        self.atualizar_medidores()?;
        Ok(RelatoRetomada {
            entrada,
            saida,
            reenvios,
        })
    }

    /// Um ciclo completo de trabalho: entrada, saída e medidores. É o que o worker do servidor
    /// (HR-20) chama a cada intervalo, ou que o laço de fundo chama sozinho.
    pub async fn ciclo(&self, transporte: &dyn TransporteSaida) -> Resultado<RelatoRetomada> {
        let mut reenvios = 0u32;
        let orgs = self.orgs()?;
        let mut entrada = RelatoDespacho::default();
        let mut saida = RelatoEnvio::default();
        for org in &orgs {
            reenvios += u32::try_from(
                self.interno
                    .store
                    .caixa_saida()
                    .contar_por_estado(*org, EstadoSaida::Enviando)?,
            )
            .unwrap_or(u32::MAX);
            entrada.somar(self.despachar(*org)?);
            saida.somar(self.enviar_pendentes(*org, transporte).await?);
        }
        self.atualizar_medidores()?;
        Ok(RelatoRetomada {
            entrada,
            saida,
            reenvios,
        })
    }

    /// Inicia um laço de fundo que chama [`Fluxo::ciclo`] a cada `intervalo_ciclo`.
    ///
    /// Precisa de um runtime tokio ativo. O handle devolvido para o laço quando solto.
    pub fn iniciar_laco(self: &Arc<Self>, transporte: Arc<dyn TransporteSaida>) -> Laco {
        let parar = Arc::new(AtomicBool::new(false));
        let sinal = Arc::clone(&parar);
        let fluxo = Arc::clone(self);
        let intervalo = self.interno.opcoes.intervalo_ciclo;
        tokio::spawn(async move {
            while !sinal.load(Ordering::Relaxed) {
                if let Err(erro) = fluxo.ciclo(transporte.as_ref()).await {
                    tracing::warn!(erro = %erro, "ciclo de entrada e saída falhou; segue no próximo");
                }
                tokio::time::sleep(intervalo).await;
            }
        });
        Laco { parar }
    }

    // ───────────────────────── Consultas ─────────────────────────

    /// Orgs existentes, paginando pelo id (v7 é ordenável por tempo).
    pub fn orgs(&self) -> Resultado<Vec<OrgId>> {
        let mut orgs = Vec::new();
        let mut apos = None;
        loop {
            let limite = 200u32;
            let pagina = self
                .interno
                .store
                .orgaos()
                .listar(Pagina { apos, limite })?;
            if pagina.is_empty() {
                break;
            }
            apos = pagina.last().map(|org| org.id.as_uuid());
            let ultimo = u32::try_from(pagina.len()).unwrap_or(u32::MAX) < limite;
            orgs.extend(pagina.into_iter().map(|org| org.id));
            if ultimo {
                break;
            }
        }
        Ok(orgs)
    }

    /// Tamanho das filas de uma org.
    pub fn estatisticas(&self, org: OrgId) -> Resultado<Estatisticas> {
        Ok(Estatisticas {
            entrada_pendente: self.interno.store.caixa_entrada().contar_pendentes(org)?,
            saida_enviando: self
                .interno
                .store
                .caixa_saida()
                .contar_por_estado(org, EstadoSaida::Enviando)?,
            saida_pendente: self
                .interno
                .store
                .caixa_saida()
                .contar_por_estado(org, EstadoSaida::Pendente)?,
            saida_falhou: self
                .interno
                .store
                .caixa_saida()
                .contar_por_estado(org, EstadoSaida::Falhou)?,
        })
    }

    /// Soma as filas de todas as orgs e publica nos medidores.
    pub fn atualizar_medidores(&self) -> Resultado<()> {
        let mut entrada = 0u64;
        let mut enviando = 0u64;
        for org in self.orgs()? {
            entrada += self.interno.store.caixa_entrada().contar_pendentes(org)?;
            enviando += self
                .interno
                .store
                .caixa_saida()
                .contar_por_estado(org, EstadoSaida::Enviando)?;
        }
        self.interno
            .metricas
            .inbox_pendentes
            .set(i64::try_from(entrada).unwrap_or(i64::MAX));
        self.interno
            .metricas
            .outbox_enviando
            .set(i64::try_from(enviando).unwrap_or(i64::MAX));
        Ok(())
    }
}

impl Interno {
    fn agora(&self) -> Instant {
        let relogio = self.relogio.lock().unwrap_or_else(|erro| erro.into_inner());
        relogio()
    }

    /// Reserva o item de entrada para este ciclo. `false` = já reservado (em voo ou em processamento).
    fn reivindicar_entrada(&self, id: InboxId) -> bool {
        let mut em_voo = self.em_voo();
        if em_voo.contains(&id) {
            return false;
        }
        em_voo.insert(id);
        true
    }

    /// Solta a reserva do item de entrada (falhou o despacho, ou o evento era inválido).
    fn liberar_entrada(&self, id: InboxId) {
        self.em_voo().remove(&id);
    }

    /// Reserva o item de saída. `false` = outro ciclo já está enviando este item.
    fn reivindicar_saida(&self, id: OutboxId) -> bool {
        let mut em_envio = self
            .em_envio
            .lock()
            .unwrap_or_else(|erro| erro.into_inner());
        if em_envio.contains(&id) {
            return false;
        }
        em_envio.insert(id);
        true
    }

    /// Solta a reserva do item de saída.
    fn liberar_saida(&self, id: OutboxId) {
        self.em_envio
            .lock()
            .unwrap_or_else(|erro| erro.into_inner())
            .remove(&id);
    }

    fn em_voo(&self) -> MutexGuard<'_, HashSet<InboxId>> {
        self.em_voo.lock().unwrap_or_else(|erro| erro.into_inner())
    }

    fn agenda_entrada(&self) -> MutexGuard<'_, HashMap<InboxId, Agenda>> {
        self.agenda_entrada
            .lock()
            .unwrap_or_else(|erro| erro.into_inner())
    }

    fn agenda_saida(&self) -> MutexGuard<'_, HashMap<OutboxId, Agenda>> {
        self.agenda_saida
            .lock()
            .unwrap_or_else(|erro| erro.into_inner())
    }
}

/// Handle do laço de fundo. Soltar (ou chamar [`Laco::parar`]) faz o laço terminar no próximo ciclo.
pub struct Laco {
    parar: Arc<AtomicBool>,
}

impl Laco {
    /// Pede o fim do laço.
    pub fn parar(&self) {
        self.parar.store(true, Ordering::Relaxed);
    }

    /// `true` depois de [`Laco::parar`].
    pub fn parado(&self) -> bool {
        self.parar.load(Ordering::Relaxed)
    }
}

impl Drop for Laco {
    fn drop(&mut self) {
        self.parar.store(true, Ordering::Relaxed);
    }
}
