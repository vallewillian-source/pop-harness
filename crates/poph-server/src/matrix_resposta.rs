//! Resposta do atendente → cliente (MX-18): o que o atendente escreve no Element chega ao
//! WhatsApp.
//!
//! Este módulo implementa a [`EntregaDeEventos`] da MX-16. Para cada evento que o Synapse entrega
//! numa transação, ele chama o classificador da MX-17 e:
//!
//! - `RespostaHumana` → acha a conversa pela sala, resolve o nome do atendente, converte o corpo
//!   para o que o WhatsApp renderiza, prefixa `*[Nome]*:`, e chama a operação atômica da MX-04
//!   (mensagem + fila de saída + conversa `transferred` + auditoria, numa escrita só), com o id do
//!   evento como chave de idempotência;
//! - `MidiaDoAtendente` → na fase 1 o arquivo **não** vai ao cliente: o atendente recebe um aviso
//!   na sala;
//! - `Ignorar` → nada, só o código do motivo no log.
//!
//! ## O que nunca acontece aqui
//!
//! - **4xx/5xx por conteúdo.** Sala desconhecida, conversa encerrada e mídia são situações
//!   previstas: respondem `200` sem mandar nada ao cliente. O único `Err` é falha transitória
//!   nossa (banco indisponível), que a MX-16 transforma em `5xx` para o Synapse reenviar.
//! - **Segundo envio no reenvio.** O Synapse pode repetir a transação; a idempotência é do
//!   `event_id` (a MX-04 devolve `Duplicada` e nada é enfileirado de novo).
//! - **Dado de cliente no log.** Loga sala, id de evento, motivo e resultado — nunca o texto.
//!
//! ## Fronteira
//!
//! A persistência entra pela porta [`PortaAtendimento`] (a mesma escolha da `PortaSalas` da MX-11):
//! a implementação real é a [`PortaDoHarness`], sobre o `Store`; os testes injetam uma porta que
//! sabe falhar, para provar o caminho do `5xx`. O cliente do Synapse é usado só para duas coisas:
//! ler o nome de exibição do atendente e escrever o aviso na sala.
//!
//! ## Quantas consultas custa achar a conversa
//!
//! O evento traz só o `room_id`, e o repositório da MX-08 lê a sala por (`org`, `room_id`) — a
//! tabela não tem leitura por sala sozinha. Enquanto essa leitura global não existir, a porta
//! percorre as orgs com o módulo ligado (`orgs_ligadas`, a mesma descoberta que o laço da MX-14
//! usa) e para na primeira que conhece a sala. O resultado fica num cache em memória **com teto**
//! ([`TETO_PADRAO_SALAS`]), então o custo é uma vez por sala, não por mensagem.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use poph_matrix::{Acao, ClienteMatrix, IdsFantasmas, ModuloMatrix, TipoMidia, classificar};
use poph_store::erro::ErroStore;
use poph_store::{RespostaHumana, RespostaHumanaNova, Store};
use poph_types::{ConversationId, OrgId};
use serde_json::{Value, json};

use crate::api::matrix_as::{EntregaDeEventos, ErroEntrega, FuturoDaEntrega};
use crate::matrix_metricas::MetricasMatrix;

/// Nome usado quando o perfil do atendente não traz nome de exibição.
pub const NOME_PADRAO_ATENDENTE: &str = "Atendente";

/// Quanto tempo o nome de exibição do atendente vale no cache.
///
/// Cinco minutos: o nome muda raramente, e um nome velho só aparece no prefixo da mensagem até a
/// próxima janela.
pub const VALIDADE_NOME_PADRAO: Duration = Duration::from_secs(5 * 60);

/// Teto do cache de nomes de atendente (por processo).
pub const TETO_PADRAO_NOMES: usize = 1024;

/// Teto do cache de sala → conversa (por processo).
pub const TETO_PADRAO_SALAS: usize = 1024;

/// Aviso na sala quando a conversa já está encerrada: nada vai ao cliente.
pub const AVISO_CONVERSA_ENCERRADA: &str =
    "Esta conversa já foi encerrada: a mensagem não foi enviada ao cliente.";

/// Aviso na sala quando o atendente manda arquivo, imagem, áudio ou vídeo (fase 1).
pub const AVISO_MIDIA_NAO_SUPORTADA: &str = "O envio de arquivos pelo Element ainda não é suportado nesta fase: o cliente não recebeu este arquivo.";

/// Marca do `txnId` do aviso de conversa encerrada.
const MARCA_AVISO_ENCERRADA: &str = "fechada";

/// Marca do `txnId` do aviso de mídia não suportada.
const MARCA_AVISO_MIDIA: &str = "midia";

/// Motivo curto devolvido à MX-16 quando o banco não respondeu.
const MOTIVO_BANCO: &str = "banco indisponível na resposta do atendente";

// ───────────────────────────── a conversa que a sala representa ─────────────────────────────

/// A conversa que a sala representa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConversaDaSala {
    /// Org dona da sala (e da conversa).
    pub org: OrgId,
    /// Conversa espelhada na sala.
    pub conversa: ConversationId,
}

// ───────────────────────────── porta de persistência ─────────────────────────────

/// O que a resposta do atendente precisa do banco.
///
/// É uma porta (e não o `Store` direto) para o teste conseguir injetar a falha de banco e provar
/// que ela vira `5xx` — nunca `4xx`, que marcaria o Application Service como fora do ar.
pub trait PortaAtendimento: Send + Sync {
    /// A conversa da sala, ou `None` quando nenhuma org ligada conhece essa sala.
    ///
    /// `Err` significa "não consegui olhar agora": vira `5xx` e o Synapse reenvia.
    fn conversa_da_sala(&self, sala: &str) -> Result<Option<ConversaDaSala>, ErroStore>;

    /// A operação atômica da MX-04.
    fn registrar_resposta(
        &self,
        alvo: ConversaDaSala,
        nova: RespostaHumanaNova<'_>,
    ) -> Result<RespostaHumana, ErroStore>;
}

/// A porta de verdade, sobre o banco do Harness.
#[derive(Debug, Clone)]
pub struct PortaDoHarness {
    store: Arc<Store>,
}

impl PortaDoHarness {
    /// Monta a porta sobre o banco.
    pub fn nova(store: Arc<Store>) -> Self {
        Self { store }
    }
}

impl PortaAtendimento for PortaDoHarness {
    fn conversa_da_sala(&self, sala: &str) -> Result<Option<ConversaDaSala>, ErroStore> {
        // Só orgs com o módulo ligado: sala de org desligada não recebe resposta nenhuma (o
        // espelhamento e o módulo inteiro estão parados para ela).
        for estado in self.store.matrix().orgs_ligadas()? {
            if let Some(gravada) = self.store.matrix().sala_por_room_id(estado.org_id, sala)? {
                return Ok(Some(ConversaDaSala {
                    org: estado.org_id,
                    conversa: gravada.conversa,
                }));
            }
        }
        Ok(None)
    }

    fn registrar_resposta(
        &self,
        alvo: ConversaDaSala,
        nova: RespostaHumanaNova<'_>,
    ) -> Result<RespostaHumana, ErroStore> {
        self.store
            .mensagens()
            .registrar_resposta_humana(alvo.org, alvo.conversa, nova)
    }
}

// ───────────────────────────── a entrega ─────────────────────────────

/// Contadores do que a entrega fez neste processo.
///
/// São números simples (a exposição em `/metrics` é da MX-26): servem para o log de fecho de
/// transação e para o teste conferir, por exemplo, que sala desconhecida não gravou nada.
#[derive(Debug, Default)]
pub struct Contadores {
    /// Respostas gravadas e enfileiradas.
    pub gravadas: AtomicU64,
    /// Reenvio do Synapse de um evento já gravado.
    pub duplicadas: AtomicU64,
    /// Eventos de sala que nenhuma org ligada conhece.
    pub salas_desconhecidas: AtomicU64,
    /// Respostas recusadas porque a conversa está encerrada.
    pub encerradas: AtomicU64,
    /// Avisos de mídia não suportada escritos na sala.
    pub avisos_de_midia: AtomicU64,
}

impl Contadores {
    fn somar(campo: &AtomicU64) {
        campo.fetch_add(1, Ordering::Relaxed);
    }
}

/// A entrega de eventos do Application Service: grava a resposta do atendente (MX-18).
pub struct RespostaDoAtendente {
    porta: Arc<dyn PortaAtendimento>,
    cliente: Option<ClienteMatrix>,
    ids: IdsFantasmas,
    salas: Mutex<CacheSimples<ConversaDaSala>>,
    nomes: Mutex<CacheDeNomes>,
    validade_nome: Duration,
    contadores: Contadores,
    /// Números do módulo (MX-26). Sem injeção, cai num registro descartável.
    metricas: Arc<MetricasMatrix>,
}

impl RespostaDoAtendente {
    /// Monta a entrega com os tetos padrão.
    pub fn novo(
        porta: Arc<dyn PortaAtendimento>,
        cliente: Option<ClienteMatrix>,
        ids: IdsFantasmas,
    ) -> Self {
        Self::com_tetos(
            porta,
            cliente,
            ids,
            TETO_PADRAO_SALAS,
            TETO_PADRAO_NOMES,
            VALIDADE_NOME_PADRAO,
        )
    }

    /// Igual a [`RespostaDoAtendente::novo`], com tetos e validade escolhidos pelo chamador.
    ///
    /// Teto `0` vale `1`: um cache que não guarda nada só multiplicaria consultas.
    pub fn com_tetos(
        porta: Arc<dyn PortaAtendimento>,
        cliente: Option<ClienteMatrix>,
        ids: IdsFantasmas,
        teto_salas: usize,
        teto_nomes: usize,
        validade_nome: Duration,
    ) -> Self {
        Self {
            porta,
            cliente,
            ids,
            salas: Mutex::new(CacheSimples::novo(teto_salas)),
            nomes: Mutex::new(CacheDeNomes::novo(teto_nomes, validade_nome)),
            validade_nome,
            contadores: Contadores::default(),
            metricas: MetricasMatrix::descartaveis(),
        }
    }

    /// Usa os números do processo (`Estado::metricas_matrix`) em vez de um registro descartável.
    pub fn com_metricas(mut self, metricas: Arc<MetricasMatrix>) -> Self {
        self.metricas = metricas;
        self
    }

    /// Conta uma falha de chamada ao Synapse: classe e, se for o limite de taxa, o `429`.
    fn contar_erro(&self, erro: &poph_matrix::ErroChamada) {
        match erro.classificacao() {
            poph_matrix::erro::Classificacao::Transitoria => {
                self.metricas.erros_transitorios.incrementar(1);
            }
            poph_matrix::erro::Classificacao::Definitiva => {
                self.metricas.erros_definitivos.incrementar(1);
            }
        }
        if matches!(erro, poph_matrix::ErroChamada::Resposta { status: 429, .. }) {
            self.metricas.respostas_429.incrementar(1);
        }
    }

    /// Monta sobre o banco do Harness, a partir do módulo já preparado (config + segredos).
    ///
    /// É o que a MX-20 chama: `RespostaDoAtendente::do_modulo(store, &modulo)` e depois
    /// `ReceptorAs::do_modulo(modulo, Arc::new(entrega))`.
    pub fn do_modulo(
        store: Arc<Store>,
        modulo: &ModuloMatrix,
    ) -> Result<Self, poph_matrix::ErroChamada> {
        let cliente = ClienteMatrix::das_partes(&modulo.config, &modulo.segredos)?;
        let ids = IdsFantasmas::da_config(&modulo.config).map_err(|erro| {
            poph_matrix::ErroChamada::Configuracao(format!("ids dos usuários virtuais: {erro}"))
        })?;
        Ok(Self::novo(
            Arc::new(PortaDoHarness::nova(store)),
            Some(cliente),
            ids,
        ))
    }

    /// Os contadores (teste e métrica).
    pub fn contadores(&self) -> &Contadores {
        &self.contadores
    }

    /// `(ocupado, teto)` do cache de salas e do cache de nomes, para métrica e teste.
    pub fn cache(&self) -> ((usize, usize), (usize, usize)) {
        let salas = {
            let guardadas = self.salas_guardadas();
            (guardadas.mapa.len(), guardadas.teto)
        };
        let nomes = {
            let guardadas = self.nomes_guardados();
            (guardadas.mapa.len(), guardadas.teto)
        };
        (salas, nomes)
    }

    /// Validade do nome de exibição em cache (teste).
    pub fn validade_do_nome(&self) -> Duration {
        self.validade_nome
    }

    /// O mutex do cache de salas nunca fica travado durante `await`; envenenado, é recuperado.
    fn salas_guardadas(&self) -> std::sync::MutexGuard<'_, CacheSimples<ConversaDaSala>> {
        self.salas
            .lock()
            .unwrap_or_else(|veneno| veneno.into_inner())
    }

    fn nomes_guardados(&self) -> std::sync::MutexGuard<'_, CacheDeNomes> {
        self.nomes
            .lock()
            .unwrap_or_else(|veneno| veneno.into_inner())
    }

    fn esquecer_sala(&self, sala: &str) {
        self.salas_guardadas().esquecer(sala);
    }

    /// A conversa da sala, pelo cache ou pela porta.
    fn conversa_da_sala(&self, sala: &str) -> Result<Option<ConversaDaSala>, ErroStore> {
        if let Some(alvo) = self.salas_guardadas().obter(sala) {
            return Ok(Some(alvo));
        }
        let Some(alvo) = self.porta.conversa_da_sala(sala)? else {
            return Ok(None);
        };
        self.salas_guardadas().registrar(sala, alvo);
        Ok(Some(alvo))
    }

    /// Nome de exibição do atendente, com cache e validade; "Atendente" quando não há.
    async fn nome_do_atendente(&self, remetente: &str) -> String {
        if let Some(nome) = self.nomes_guardados().obter(remetente) {
            return nome;
        }
        let Some(cliente) = &self.cliente else {
            return NOME_PADRAO_ATENDENTE.to_owned();
        };
        match cliente.perfil(remetente).await {
            Ok(perfil) => {
                let nome = nome_util(perfil.displayname.as_deref());
                self.nomes_guardados().registrar(remetente, &nome);
                nome
            }
            Err(erro) => {
                // Sem nome não se perde a resposta: o prefixo cai no padrão e a próxima mensagem
                // tenta o perfil de novo.
                self.contar_erro(&erro);
                tracing::debug!(
                    target: "poph_server::matrix_resposta",
                    classificacao = ?erro.classificacao(),
                    "não consegui ler o perfil do atendente; usando o nome padrão"
                );
                NOME_PADRAO_ATENDENTE.to_owned()
            }
        }
    }

    /// Escreve um aviso na sala, em nome de quem mandou o evento (o único membro que a fase 1
    /// garante na sala além do fantasma do cliente).
    ///
    /// Aviso é melhor-esforço: o atendente já viu que a mensagem não foi, e derrubar a transação
    /// por causa do aviso faria o Synapse reenviar a resposta do atendente de novo.
    async fn avisar(&self, sala: &str, remetente: &str, marca: &str, evento_id: &str, corpo: &str) {
        let Some(cliente) = &self.cliente else {
            return;
        };
        let txn = txn_do_evento(marca, evento_id);
        if let Err(erro) = cliente
            .enviar_evento(
                remetente,
                sala,
                "m.room.message",
                &txn,
                &json!({ "msgtype": "m.notice", "body": corpo }),
            )
            .await
        {
            self.contar_erro(&erro);
            tracing::debug!(
                target: "poph_server::matrix_resposta",
                motivo = ?erro.classificacao(),
                "não consegui escrever o aviso na sala"
            );
        }
    }

    async fn responder(
        &self,
        transacao: &str,
        sala: &str,
        evento_id: &str,
        remetente: &str,
        texto: &str,
    ) -> Result<(), ErroEntrega> {
        let alvo = match self.conversa_da_sala(sala) {
            Ok(Some(alvo)) => alvo,
            Ok(None) => {
                Contadores::somar(&self.contadores.salas_desconhecidas);
                self.metricas.salas_desconhecidas.incrementar(1);
                tracing::info!(
                    target: "poph_server::matrix_resposta",
                    transacao,
                    sala,
                    "evento de sala que nenhuma org ligada conhece: ignorado"
                );
                return Ok(());
            }
            Err(_) => {
                tracing::error!(
                    target: "poph_server::matrix_resposta",
                    transacao,
                    sala,
                    "não consegui achar a conversa da sala agora"
                );
                return Err(ErroEntrega::novo(MOTIVO_BANCO));
            }
        };

        let nome = self.nome_do_atendente(remetente).await;
        let enviado = texto_enfileirado(&nome, texto);
        let nova = RespostaHumanaNova {
            evento_id,
            texto_bruto: texto,
            texto_enviado: &enviado,
            autor_id: remetente,
            autor_nome: &nome,
        };

        match self.porta.registrar_resposta(alvo, nova) {
            Ok(RespostaHumana::Gravada(_)) => {
                Contadores::somar(&self.contadores.gravadas);
                self.metricas.respostas_enfileiradas.incrementar(1);
                tracing::info!(
                    target: "poph_server::matrix_resposta",
                    org = %alvo.org,
                    conversa = %alvo.conversa,
                    transacao,
                    sala,
                    evento = evento_id,
                    "resposta do atendente enfileirada para o cliente"
                );
                Ok(())
            }
            Ok(RespostaHumana::Duplicada) => {
                Contadores::somar(&self.contadores.duplicadas);
                self.metricas.respostas_duplicadas.incrementar(1);
                tracing::debug!(
                    target: "poph_server::matrix_resposta",
                    transacao,
                    evento = evento_id,
                    "reenvio do Synapse de uma resposta já gravada"
                );
                Ok(())
            }
            Ok(RespostaHumana::ConversaFechada) => {
                Contadores::somar(&self.contadores.encerradas);
                self.metricas.respostas_conversa_encerrada.incrementar(1);
                tracing::info!(
                    target: "poph_server::matrix_resposta",
                    org = %alvo.org,
                    conversa = %alvo.conversa,
                    transacao,
                    sala,
                    evento = evento_id,
                    "conversa encerrada: resposta não enviada"
                );
                self.avisar(
                    sala,
                    remetente,
                    MARCA_AVISO_ENCERRADA,
                    evento_id,
                    AVISO_CONVERSA_ENCERRADA,
                )
                .await;
                Ok(())
            }
            Ok(RespostaHumana::ConversaInexistente) => {
                // A sala aponta para uma conversa que sumiu (ou é de outra org): esquecer o cache
                // evita insistir na linha errada, e a próxima mensagem procura de novo.
                self.esquecer_sala(sala);
                Contadores::somar(&self.contadores.salas_desconhecidas);
                self.metricas.salas_desconhecidas.incrementar(1);
                tracing::info!(
                    target: "poph_server::matrix_resposta",
                    transacao,
                    sala,
                    "a sala aponta para uma conversa que não existe nesta org: ignorado"
                );
                Ok(())
            }
            Err(_) => {
                tracing::error!(
                    target: "poph_server::matrix_resposta",
                    org = %alvo.org,
                    conversa = %alvo.conversa,
                    transacao,
                    sala,
                    motivo = "banco_indisponivel",
                    "não consegui gravar a resposta do atendente agora"
                );
                Err(ErroEntrega::novo(MOTIVO_BANCO))
            }
        }
    }

    async fn guardar_midia(
        &self,
        transacao: &str,
        sala: &str,
        evento_id: &str,
        remetente: &str,
        tipo: TipoMidia,
    ) {
        // Mídia não vira resposta nesta fase, e a conversa nem precisa existir: o aviso é para o
        // atendente, na sala.
        Contadores::somar(&self.contadores.avisos_de_midia);
        self.metricas.avisos_de_midia.incrementar(1);
        tracing::info!(
            target: "poph_server::matrix_resposta",
            transacao,
            sala,
            evento = evento_id,
            tipo = tipo.msgtype(),
            "mídia do atendente: avisando que ainda não é suportada"
        );
        self.avisar(
            sala,
            remetente,
            MARCA_AVISO_MIDIA,
            evento_id,
            AVISO_MIDIA_NAO_SUPORTADA,
        )
        .await;
    }

    async fn processar(&self, transacao: &str, eventos: Vec<Value>) -> Result<(), ErroEntrega> {
        for evento in &eventos {
            match classificar(&self.ids, evento) {
                Acao::RespostaHumana {
                    sala,
                    evento_id,
                    remetente,
                    texto,
                } => {
                    self.responder(transacao, &sala, &evento_id, &remetente, &texto)
                        .await?;
                }
                Acao::MidiaDoAtendente {
                    sala,
                    evento_id,
                    remetente,
                    tipo,
                } => {
                    self.guardar_midia(transacao, &sala, &evento_id, &remetente, tipo)
                        .await;
                }
                Acao::Ignorar { motivo } => {
                    tracing::debug!(
                        target: "poph_server::matrix_resposta",
                        transacao,
                        motivo = motivo.codigo(),
                        "evento do Matrix ignorado"
                    );
                }
            }
        }
        Ok(())
    }
}

impl EntregaDeEventos for RespostaDoAtendente {
    fn entregar(&self, transacao: &str, eventos: Vec<Value>) -> FuturoDaEntrega<'_> {
        // O id da transação vira dono: o futuro devolvido só empresta `self` (é o que o trait
        // permite), e `transacao` chega emprestada de quem chamou.
        let transacao = transacao.to_owned();
        Box::pin(async move { self.processar(&transacao, eventos).await })
    }
}

impl std::fmt::Debug for RespostaDoAtendente {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RespostaDoAtendente")
            .field("cliente", &self.cliente)
            .field("ids", &self.ids)
            .field("validade_nome", &self.validade_nome)
            .finish_non_exhaustive()
    }
}

// ───────────────────────────── texto que vai ao cliente ─────────────────────────────

/// Texto que entra na fila de saída para o cliente: o nome do atendente em negrito e o corpo
/// **exatamente como ele escreveu**.
///
/// ## Por que a conversão não é feita aqui
///
/// O envio (`ClienteEvolution::enviar_texto`) já converte tudo o que sai da fila com
/// [`poph_evolution::texto::para_whatsapp`] — é o mesmo caminho de toda mensagem do Harness. Se este módulo convertesse
/// antes, o texto seria convertido **duas vezes**: `**negrito**` viraria `*negrito*` na primeira e
/// `_negrito_` (itálico) na segunda, e o próprio prefixo `*[Nome]*` seria lido como itálico. Por
/// isso o prefixo vai em Markdown (`**[Nome]**:`), que a conversão única transforma no
/// `*[Nome]*:` que o cliente vê.
///
/// O corte também é o do canal: [`poph_evolution::texto::para_whatsapp`] termina chamando [`poph_evolution::texto::cortar`], então uma mensagem
/// acima de [`poph_evolution::LIMITE_TEXTO`] chega ao cliente cortada com `…`, numa mensagem só —
/// nunca dividida em duas.
pub fn texto_enfileirado(nome: &str, texto: &str) -> String {
    format!("**[{}]**:\n{texto}", nome_para_prefixo(nome))
}

/// Nome de exibição utilizável no prefixo: sem controle, sem marcador de formatação do canal
/// (`*`, `_`, `~`, crase) e nunca vazio.
///
/// Os marcadores saem porque o prefixo vai em Markdown: um `*` no meio do nome fecharia o
/// negrito cedo e o cliente veria `*Ana*]*` em vez do nome.
fn nome_para_prefixo(nome: &str) -> String {
    let limpo: String = nome
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '*' | '_' | '~' | '`') {
                ' '
            } else {
                c
            }
        })
        .collect();
    let limpo = limpo.trim();
    if limpo.is_empty() {
        return NOME_PADRAO_ATENDENTE.to_owned();
    }
    // Um nome enorme não pode ocupar o prefixo inteiro.
    if limpo.chars().count() > 80 {
        return limpo
            .chars()
            .take(80)
            .collect::<String>()
            .trim_end()
            .to_owned();
    }
    limpo.to_owned()
}

/// Nome utilizável quando vem de um `Option` (perfil do Synapse, por exemplo).
fn nome_util(nome: Option<&str>) -> String {
    nome_para_prefixo(nome.unwrap_or_default())
}

/// `txnId` determinístico do aviso: repetir o aviso do mesmo evento não duplica (P7).
fn txn_do_evento(marca: &str, evento_id: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in evento_id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("ph_{marca}_{hash:016x}")
}

// ───────────────────────────── caches em memória com teto ─────────────────────────────

/// Mapa FIFO com teto: quando enche, o mais antigo sai.
///
/// Esquecer uma entrada custa uma consulta a mais (nunca um erro), então o teto pode ser pequeno.
#[derive(Debug)]
struct CacheSimples<V> {
    teto: usize,
    ordem: VecDeque<String>,
    mapa: HashMap<String, V>,
}

impl<V: Copy> CacheSimples<V> {
    fn novo(teto: usize) -> Self {
        Self {
            teto: teto.max(1),
            ordem: VecDeque::new(),
            mapa: HashMap::new(),
        }
    }

    fn obter(&self, chave: &str) -> Option<V> {
        self.mapa.get(chave).copied()
    }

    fn registrar(&mut self, chave: &str, valor: V) {
        if self.mapa.contains_key(chave) {
            return;
        }
        while self.ordem.len() >= self.teto {
            match self.ordem.pop_front() {
                Some(antiga) => {
                    self.mapa.remove(&antiga);
                }
                None => break,
            }
        }
        self.ordem.push_back(chave.to_owned());
        self.mapa.insert(chave.to_owned(), valor);
    }

    fn esquecer(&mut self, chave: &str) {
        if self.mapa.remove(chave).is_some() {
            self.ordem.retain(|guardada| guardada != chave);
        }
    }
}

/// Cache do nome de exibição do atendente, com teto e validade.
#[derive(Debug)]
struct CacheDeNomes {
    teto: usize,
    validade: Duration,
    ordem: VecDeque<String>,
    mapa: HashMap<String, (String, Instant)>,
}

impl CacheDeNomes {
    fn novo(teto: usize, validade: Duration) -> Self {
        Self {
            teto: teto.max(1),
            validade,
            ordem: VecDeque::new(),
            mapa: HashMap::new(),
        }
    }

    fn obter(&self, chave: &str) -> Option<String> {
        let (nome, quando) = self.mapa.get(chave)?;
        if quando.elapsed() > self.validade {
            return None;
        }
        Some(nome.clone())
    }

    fn registrar(&mut self, chave: &str, nome: &str) {
        if let Some((guardado, quando)) = self.mapa.get_mut(chave) {
            *guardado = nome.to_owned();
            *quando = Instant::now();
            return;
        }
        while self.ordem.len() >= self.teto {
            match self.ordem.pop_front() {
                Some(antiga) => {
                    self.mapa.remove(&antiga);
                }
                None => break,
            }
        }
        self.ordem.push_back(chave.to_owned());
        self.mapa
            .insert(chave.to_owned(), (nome.to_owned(), Instant::now()));
    }
}

#[cfg(test)]
mod testes {
    use poph_evolution::para_whatsapp;

    use super::*;

    fn alvo() -> ConversaDaSala {
        ConversaDaSala {
            org: OrgId::generate(),
            conversa: ConversationId::generate(),
        }
    }

    #[test]
    fn cache_de_salas_respeita_o_teto_e_esquece_o_mais_antigo() {
        let mut cache = CacheSimples::novo(2);
        cache.registrar("!a", alvo());
        cache.registrar("!b", alvo());
        cache.registrar("!c", alvo());
        assert_eq!(cache.mapa.len(), 2);
        assert!(cache.obter("!a").is_none(), "o mais antigo saiu");
        assert!(cache.obter("!b").is_some());
        assert!(cache.obter("!c").is_some());
    }

    #[test]
    fn cache_de_salas_esquece_uma_sala() {
        let mut cache = CacheSimples::novo(4);
        cache.registrar("!a", alvo());
        cache.esquecer("!a");
        assert!(cache.obter("!a").is_none());
        assert!(cache.ordem.is_empty());
    }

    #[test]
    fn cache_de_nomes_respeita_o_teto() {
        let mut cache = CacheDeNomes::novo(2, Duration::from_secs(60));
        cache.registrar("@a:s", "Ana");
        cache.registrar("@b:s", "Bia");
        cache.registrar("@c:s", "Caio");
        assert_eq!(cache.mapa.len(), 2);
        assert!(cache.obter("@a:s").is_none());
        assert_eq!(cache.obter("@c:s").as_deref(), Some("Caio"));
    }

    #[test]
    fn cache_de_nomes_vence_pela_validade() {
        let mut cache = CacheDeNomes::novo(4, Duration::ZERO);
        cache.registrar("@a:s", "Ana");
        assert!(cache.obter("@a:s").is_none(), "validade zero vence na hora");
    }

    #[test]
    fn cache_de_nomes_atualiza_o_nome_da_mesma_pessoa() {
        let mut cache = CacheDeNomes::novo(4, Duration::from_secs(60));
        cache.registrar("@a:s", "Ana");
        cache.registrar("@a:s", "Ana Souza");
        assert_eq!(cache.obter("@a:s").as_deref(), Some("Ana Souza"));
        assert_eq!(cache.mapa.len(), 1);
    }

    #[test]
    fn o_cliente_recebe_o_prefixo_e_o_corpo_convertido_uma_vez_so() {
        let fila = texto_enfileirado("Ana", "olá");
        assert_eq!(fila, "**[Ana]**:\nolá");
        assert_eq!(para_whatsapp(&fila), "*[Ana]*:\nolá");
    }

    #[test]
    fn corpo_com_markdown_nao_e_convertido_duas_vezes() {
        // `**negrito**` tem de virar negrito no WhatsApp, não itálico: quem converte é o envio.
        let fila = texto_enfileirado("Ana", "**atenção** e _ênfase_");
        let no_cliente = para_whatsapp(&fila);
        assert_eq!(no_cliente, "*[Ana]*:\n*atenção* e _ênfase_");
    }

    #[test]
    fn texto_acima_do_limite_e_cortado_pelo_canal_numa_mensagem_so() {
        let enorme = "a".repeat(5000);
        let no_cliente = para_whatsapp(&texto_enfileirado("Ana", &enorme));
        assert!(no_cliente.chars().count() <= poph_evolution::LIMITE_TEXTO);
        assert!(no_cliente.starts_with("*[Ana]*:\n"));
        assert!(
            no_cliente.ends_with('…'),
            "corta com reticências, não divide"
        );
    }

    #[test]
    fn nome_do_prefixo_nao_quebra_o_negrito_nem_fica_vazio() {
        assert_eq!(nome_util(None), NOME_PADRAO_ATENDENTE);
        assert_eq!(nome_util(Some("   ")), NOME_PADRAO_ATENDENTE);
        assert_eq!(nome_util(Some("Ana\nSouza")), "Ana Souza");
        // Marcador de formatação no nome não pode fechar o negrito antes da hora.
        assert_eq!(
            para_whatsapp(&texto_enfileirado("Ana*Souza", "oi")),
            "*[Ana Souza]*:\noi"
        );
        let longo = "n".repeat(200);
        assert_eq!(nome_util(Some(&longo)).chars().count(), 80);
    }

    #[test]
    fn txn_do_aviso_e_estavel_e_distinto_por_marca() {
        assert_eq!(
            txn_do_evento("midia", "$a:s"),
            txn_do_evento("midia", "$a:s")
        );
        assert_ne!(
            txn_do_evento("midia", "$a:s"),
            txn_do_evento("fechada", "$a:s")
        );
        assert_ne!(
            txn_do_evento("midia", "$a:s"),
            txn_do_evento("midia", "$b:s")
        );
    }
}
