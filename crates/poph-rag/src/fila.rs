//! Fila de embedding com lote e limite de simultaneidade (HR-15).
//!
//! Como funciona:
//! - Cada pedido ([`FilaEmbedding::embedir`]) passa por duas vagas antes de entrar na fila: uma
//!   da org (`simultaneos_por_org`) e uma global (`simultaneos_total`). Org saturada espera na
//!   própria vaga e não ocupa a global, então não atrasa as outras orgs.
//! - Na fila, os trechos ficam separados por org. Cada lote pega um trecho de cada org por vez
//!   (rodízio), até `tamanho_lote`. Uma org com muitos trechos não empurra as demais para trás.
//! - Um único laço processa os lotes, um de cada vez: a sessão do ONNX Runtime não roda duas
//!   vezes ao mesmo tempo, e dois lotes em paralelo só dividiriam os mesmos núcleos.
//! - Pedido que saiu (quem chamou desistiu) tem os trechos pulados antes do próximo lote.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use poph_types::OrgId;
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, mpsc};

use crate::embedder::{Embedder, IdentidadeModelo};
use crate::erro::ErroRag;
use crate::metricas::MetricasRag;

/// Limites da fila. Os padrões são propostas da HR-15 (D-09 não fixa estes números).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigFila {
    /// Trechos por lote. Padrão 32, como medido no spike (RELATORIO_HR02 §9).
    pub tamanho_lote: usize,
    /// Tempo que o lote espera para encher antes de sair. Padrão 10 ms.
    pub janela: Duration,
    /// Pedidos simultâneos de uma mesma org. Padrão 4 (proposta).
    pub simultaneos_por_org: usize,
    /// Pedidos simultâneos no total da instância. Padrão 32 (proposta).
    pub simultaneos_total: usize,
}

impl Default for ConfigFila {
    fn default() -> Self {
        Self {
            tamanho_lote: 32,
            janela: Duration::from_millis(10),
            simultaneos_por_org: 4,
            simultaneos_total: 32,
        }
    }
}

impl ConfigFila {
    /// Recusa valores que travariam a fila (zero em tamanho de lote ou em vagas).
    pub fn validar(&self) -> Result<(), ErroRag> {
        if self.tamanho_lote == 0 {
            return Err(ErroRag::Configuracao(
                "tamanho do lote deve ser maior que zero".to_owned(),
            ));
        }
        if self.simultaneos_por_org == 0 || self.simultaneos_total == 0 {
            return Err(ErroRag::Configuracao(
                "os limites de simultaneidade devem ser maiores que zero".to_owned(),
            ));
        }
        Ok(())
    }
}

type Resposta = mpsc::UnboundedSender<(usize, Result<Vec<f32>, ErroRag>)>;

/// Vagas de cada org: o semáforo de pedidos e a contagem de pedidos em andamento.
type VagasPorOrg = HashMap<OrgId, (Arc<Semaphore>, Arc<Contagem>)>;

struct Item {
    texto: String,
    posicao: usize,
    resposta: Resposta,
}

#[derive(Default)]
struct Pendentes {
    por_org: HashMap<OrgId, VecDeque<Item>>,
    rodizio: VecDeque<OrgId>,
    total: usize,
}

impl Pendentes {
    fn adicionar(&mut self, org: OrgId, itens: Vec<Item>) {
        if itens.is_empty() {
            return;
        }
        self.total += itens.len();
        let fila = self.por_org.entry(org).or_default();
        let estava_vazia = fila.is_empty();
        fila.extend(itens);
        if estava_vazia {
            self.rodizio.push_back(org);
        }
    }

    /// Tira até `max` trechos, um de cada org por vez. Pula os de pedidos que já saíram.
    fn tirar_lote(&mut self, max: usize) -> Vec<Item> {
        let mut lote = Vec::new();
        while lote.len() < max {
            let Some(org) = self.rodizio.pop_front() else {
                break;
            };
            let Some(fila) = self.por_org.get_mut(&org) else {
                continue;
            };
            let mut proximo = None;
            while let Some(candidato) = fila.pop_front() {
                self.total -= 1;
                if !candidato.resposta.is_closed() {
                    proximo = Some(candidato);
                    break;
                }
            }
            let ainda_tem = !fila.is_empty();
            if ainda_tem {
                self.rodizio.push_back(org);
            } else {
                self.por_org.remove(&org);
            }
            if let Some(item) = proximo {
                lote.push(item);
            }
        }
        lote
    }
}

#[derive(Default)]
struct Contagem {
    atual: AtomicUsize,
    pico: AtomicUsize,
}

impl Contagem {
    fn entrar(&self) {
        let agora = self.atual.fetch_add(1, Ordering::SeqCst) + 1;
        self.pico.fetch_max(agora, Ordering::SeqCst);
    }

    fn sair(&self) {
        self.atual.fetch_sub(1, Ordering::SeqCst);
    }
}

struct Compartilhado {
    pendentes: Mutex<Pendentes>,
    aviso: Notify,
    encerrada: AtomicBool,
    metricas: Option<MetricasRag>,
}

/// Fila de embedding. Guarde em `Arc` e compartilhe entre as orgs.
///
/// Precisa de um runtime do tokio ativo quando é criada ([`FilaEmbedding::iniciar`]).
pub struct FilaEmbedding {
    compartilhado: Arc<Compartilhado>,
    simultaneos_por_org: usize,
    vagas_total: Arc<Semaphore>,
    contagem_total: Arc<Contagem>,
    vagas_org: Mutex<VagasPorOrg>,
}

/// Pedido admitido. Ao sair de escopo, libera as duas vagas e a contagem.
struct Admissao {
    _org: OwnedSemaphorePermit,
    _total: OwnedSemaphorePermit,
    contagem_org: Arc<Contagem>,
    contagem_total: Arc<Contagem>,
}

impl Drop for Admissao {
    fn drop(&mut self) {
        self.contagem_org.sair();
        self.contagem_total.sair();
    }
}

impl FilaEmbedding {
    /// Cria a fila e o laço que processa os lotes com `embedder`.
    pub fn iniciar(
        embedder: Arc<dyn Embedder>,
        config: ConfigFila,
        metricas: Option<MetricasRag>,
    ) -> Result<Self, ErroRag> {
        config.validar()?;
        let compartilhado = Arc::new(Compartilhado {
            pendentes: Mutex::new(Pendentes::default()),
            aviso: Notify::new(),
            encerrada: AtomicBool::new(false),
            metricas,
        });
        tokio::spawn(laco(embedder, Arc::clone(&compartilhado), config.clone()));
        Ok(Self {
            compartilhado,
            simultaneos_por_org: config.simultaneos_por_org,
            vagas_total: Arc::new(Semaphore::new(config.simultaneos_total)),
            contagem_total: Arc::new(Contagem::default()),
            vagas_org: Mutex::new(HashMap::new()),
        })
    }

    /// Gera os vetores de `textos` para a org dada, na mesma ordem. Lista vazia devolve vazia.
    ///
    /// Espera a vaga da org e a global; depois, os trechos entram na fila e o pedido só volta
    /// quando todos os lotes que contêm os seus trechos terminarem.
    pub async fn embedir(&self, org: OrgId, textos: Vec<String>) -> Result<Vec<Vec<f32>>, ErroRag> {
        if textos.is_empty() {
            return Ok(Vec::new());
        }
        let total = textos.len();
        let _admissao = self.admitir(org).await?;

        let (tx, mut rx) = mpsc::unbounded_channel();
        let itens: Vec<Item> = textos
            .into_iter()
            .enumerate()
            .map(|(posicao, texto)| Item {
                texto,
                posicao,
                resposta: tx.clone(),
            })
            .collect();
        drop(tx);
        {
            let mut pendentes = trancar(&self.compartilhado.pendentes);
            if self.compartilhado.encerrada.load(Ordering::SeqCst) {
                return Err(ErroRag::FilaEncerrada);
            }
            pendentes.adicionar(org, itens);
        }
        self.compartilhado.aviso.notify_one();
        if let Some(metricas) = &self.compartilhado.metricas {
            metricas
                .trechos_pendentes
                .set(self.trechos_pendentes() as i64);
        }

        let mut saida: Vec<Option<Vec<f32>>> = vec![None; total];
        for _ in 0..total {
            match rx.recv().await {
                Some((posicao, Ok(vetor))) => saida[posicao] = Some(vetor),
                Some((_, Err(erro))) => return Err(erro),
                None => return Err(ErroRag::FilaEncerrada),
            }
        }
        saida
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or(ErroRag::FilaEncerrada)
    }

    /// Trechos esperando na fila agora.
    pub fn trechos_pendentes(&self) -> usize {
        trancar(&self.compartilhado.pendentes).total
    }

    /// Pedidos admitidos agora (todas as orgs).
    pub fn pedidos_em_andamento(&self) -> usize {
        self.contagem_total.atual.load(Ordering::SeqCst)
    }

    /// Maior número de pedidos admitidos ao mesmo tempo (todas as orgs), desde o início.
    pub fn pico_pedidos(&self) -> usize {
        self.contagem_total.pico.load(Ordering::SeqCst)
    }

    /// Maior número de pedidos admitidos ao mesmo tempo, para uma org, desde o início.
    pub fn pico_pedidos_da_org(&self, org: OrgId) -> usize {
        match trancar_vagas(&self.vagas_org).get(&org) {
            Some((_, contagem)) => contagem.pico.load(Ordering::SeqCst),
            None => 0,
        }
    }

    async fn admitir(&self, org: OrgId) -> Result<Admissao, ErroRag> {
        let (semaforo_org, contagem_org) = self.vaga_da_org(org);
        let da_org = semaforo_org
            .acquire_owned()
            .await
            .map_err(|_| ErroRag::FilaEncerrada)?;
        let total = Arc::clone(&self.vagas_total)
            .acquire_owned()
            .await
            .map_err(|_| ErroRag::FilaEncerrada)?;
        contagem_org.entrar();
        self.contagem_total.entrar();
        Ok(Admissao {
            _org: da_org,
            _total: total,
            contagem_org,
            contagem_total: Arc::clone(&self.contagem_total),
        })
    }

    fn vaga_da_org(&self, org: OrgId) -> (Arc<Semaphore>, Arc<Contagem>) {
        let mut mapa = trancar_vagas(&self.vagas_org);
        let (semaforo, contagem) = mapa.entry(org).or_insert_with(|| {
            (
                Arc::new(Semaphore::new(self.simultaneos_por_org)),
                Arc::new(Contagem::default()),
            )
        });
        (Arc::clone(semaforo), Arc::clone(contagem))
    }
}

impl Drop for FilaEmbedding {
    fn drop(&mut self) {
        encerrar(&self.compartilhado);
    }
}

/// Marca a fila como encerrada e descarta os pendentes, para quem espera receber erro e não travar.
fn encerrar(compartilhado: &Compartilhado) {
    let pendentes = {
        let mut guarda = trancar(&compartilhado.pendentes);
        compartilhado.encerrada.store(true, Ordering::SeqCst);
        std::mem::take(&mut *guarda)
    };
    drop(pendentes);
    compartilhado.aviso.notify_one();
}

/// Ao sair do laço (por encerramento ou pânico), a fila se encerra sozinha.
struct GuardaLaco(Arc<Compartilhado>);

impl Drop for GuardaLaco {
    fn drop(&mut self) {
        encerrar(&self.0);
    }
}

async fn laco(embedder: Arc<dyn Embedder>, compartilhado: Arc<Compartilhado>, config: ConfigFila) {
    let _guarda = GuardaLaco(Arc::clone(&compartilhado));
    loop {
        // Espera ao menos um trecho, ou o encerramento.
        loop {
            if compartilhado.encerrada.load(Ordering::SeqCst) {
                return;
            }
            if trancar(&compartilhado.pendentes).total > 0 {
                break;
            }
            compartilhado.aviso.notified().await;
        }

        // Dá à janela o tempo de encher o lote.
        let prazo = Instant::now() + config.janela;
        loop {
            let total = trancar(&compartilhado.pendentes).total;
            if total >= config.tamanho_lote || compartilhado.encerrada.load(Ordering::SeqCst) {
                break;
            }
            let restante = prazo.saturating_duration_since(Instant::now());
            if restante.is_zero() {
                break;
            }
            let _ = tokio::time::timeout(restante, compartilhado.aviso.notified()).await;
        }

        let (lote, restantes) = {
            let mut pendentes = trancar(&compartilhado.pendentes);
            let lote = pendentes.tirar_lote(config.tamanho_lote);
            (lote, pendentes.total)
        };
        if let Some(metricas) = &compartilhado.metricas {
            metricas.trechos_pendentes.set(restantes as i64);
        }
        if lote.is_empty() {
            continue;
        }
        processar(embedder.as_ref(), &compartilhado, lote).await;
    }
}

async fn processar(embedder: &dyn Embedder, compartilhado: &Compartilhado, lote: Vec<Item>) {
    let inicio = Instant::now();
    let mut textos = Vec::with_capacity(lote.len());
    let mut destinos = Vec::with_capacity(lote.len());
    for item in lote {
        textos.push(item.texto);
        destinos.push((item.posicao, item.resposta));
    }
    let quantidade = textos.len();
    let resultado = embedder.embedir_lote(&textos).await;
    let segundos = inicio.elapsed().as_secs_f64();

    match resultado {
        Ok(vetores) if vetores.len() == quantidade => {
            // Métricas antes de responder: quem recebe a resposta já vê o lote contado.
            if let Some(metricas) = &compartilhado.metricas {
                metricas.registrar_lote(quantidade, segundos);
            }
            for ((posicao, resposta), vetor) in destinos.into_iter().zip(vetores) {
                let _ = resposta.send((posicao, Ok(vetor)));
            }
        }
        Ok(vetores) => {
            let erro = ErroRag::Inferencia(format!(
                "esperava {quantidade} vetores e recebi {}",
                vetores.len()
            ));
            responder_erro(destinos, erro, compartilhado);
        }
        Err(erro) => responder_erro(destinos, erro, compartilhado),
    }
}

fn responder_erro(destinos: Vec<(usize, Resposta)>, erro: ErroRag, compartilhado: &Compartilhado) {
    if let Some(metricas) = &compartilhado.metricas {
        metricas.registrar_erro();
    }
    for (posicao, resposta) in destinos {
        let _ = resposta.send((posicao, Err(erro.clone())));
    }
}

fn trancar(pendentes: &Mutex<Pendentes>) -> MutexGuard<'_, Pendentes> {
    match pendentes.lock() {
        Ok(guarda) => guarda,
        Err(veneno) => veneno.into_inner(),
    }
}

fn trancar_vagas(vagas: &Mutex<VagasPorOrg>) -> MutexGuard<'_, VagasPorOrg> {
    match vagas.lock() {
        Ok(guarda) => guarda,
        Err(veneno) => veneno.into_inner(),
    }
}

/// `Embedder` que passa pela [`FilaEmbedding`] presa a uma org (HR-17).
///
/// A fila existe para uma org não ocupar o modelo de todas as outras. O [`Indice`] só conhece o
/// contrato `Embedder`, sem org; este adaptador fecha a lacuna na reindexação em lote: cada trecho
/// entra na vaga da org, e o limite de pedidos simultâneos da HR-15 vale de verdade.
///
/// A busca de uma única consulta continua indo direto ao modelo (é um texto só: passar pela fila só
/// acrescentaria a janela de lote à latência do turno).
///
/// [`Indice`]: crate::Indice
pub struct EmbedderDaFila {
    fila: Arc<FilaEmbedding>,
    org: OrgId,
    identidade: IdentidadeModelo,
    /// Runtime em que o laço da fila foi criado. Fica vivo enquanto o adaptador existir: se ele
    /// fosse derrubado, a fila pararia de responder no meio da reindexação.
    _runtime: Option<Arc<tokio::runtime::Runtime>>,
}

impl EmbedderDaFila {
    /// Embrulha a fila para uma org. `identidade` é a do modelo por trás da fila.
    pub fn novo(fila: Arc<FilaEmbedding>, org: OrgId, identidade: IdentidadeModelo) -> Self {
        Self {
            fila,
            org,
            identidade,
            _runtime: None,
        }
    }

    /// Como [`EmbedderDaFila::novo`], mantendo vivo o runtime em que a fila foi criada.
    ///
    /// É o que a CLI usa: ela cria o runtime da fila no próprio adaptador, e o adaptador (que vive
    /// dentro do índice) leva o runtime junto até o fim da reindexação.
    pub fn com_runtime(
        fila: Arc<FilaEmbedding>,
        org: OrgId,
        identidade: IdentidadeModelo,
        runtime: Arc<tokio::runtime::Runtime>,
    ) -> Self {
        Self {
            fila,
            org,
            identidade,
            _runtime: Some(runtime),
        }
    }
}

impl Embedder for EmbedderDaFila {
    fn identidade(&self) -> &IdentidadeModelo {
        &self.identidade
    }

    fn embedir_lote<'a>(&'a self, textos: &'a [String]) -> crate::embedder::FuturoLote<'a> {
        let fila = Arc::clone(&self.fila);
        let org = self.org;
        let textos = textos.to_vec();
        Box::pin(async move { fila.embedir(org, textos).await })
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn item(texto: &str, posicao: usize, tx: &Resposta) -> Item {
        Item {
            texto: texto.to_owned(),
            posicao,
            resposta: tx.clone(),
        }
    }

    #[tokio::test]
    async fn adaptador_embeda_pela_fila_e_respeita_a_vaga_da_org() {
        use crate::fake::EmbedderFake;

        let fake = Arc::new(EmbedderFake::new().com_atraso(Duration::from_millis(20)));
        let identidade = fake.identidade().clone();
        let embedder: Arc<dyn Embedder> = Arc::clone(&fake) as Arc<dyn Embedder>;
        let fila = Arc::new(
            FilaEmbedding::iniciar(
                Arc::clone(&embedder),
                ConfigFila {
                    simultaneos_por_org: 1,
                    simultaneos_total: 4,
                    ..ConfigFila::default()
                },
                None,
            )
            .expect("fila"),
        );
        let org = OrgId::generate();
        let adaptador = Arc::new(EmbedderDaFila::novo(
            Arc::clone(&fila),
            org,
            identidade.clone(),
        ));
        assert_eq!(
            adaptador.identidade(),
            &identidade,
            "identidade é a do modelo"
        );

        let a = vec!["primeiro".to_owned()];
        let b = vec!["segundo".to_owned()];
        let c = vec!["terceiro".to_owned()];
        let (ra, rb, rc) = tokio::join!(
            adaptador.embedir_lote(&a),
            adaptador.embedir_lote(&b),
            adaptador.embedir_lote(&c)
        );
        let ra = ra.expect("vetores");
        let rb = rb.expect("vetores");
        let rc = rc.expect("vetores");
        assert_eq!(ra[0], crate::fake::vetor_fake("primeiro"));
        assert_eq!(rb[0], crate::fake::vetor_fake("segundo"));
        assert_eq!(rc[0], crate::fake::vetor_fake("terceiro"));
        assert_eq!(
            fila.pico_pedidos_da_org(org),
            1,
            "a vaga da org é respeitada: um pedido por vez"
        );
        assert_eq!(fila.pedidos_em_andamento(), 0, "as vagas voltaram");
    }

    #[test]
    fn lote_faz_rodizio_entre_orgs() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let (a, b) = (OrgId::generate(), OrgId::generate());
        let mut pendentes = Pendentes::default();
        pendentes.adicionar(a, (0..6).map(|i| item("a", i, &tx)).collect());
        pendentes.adicionar(b, (0..2).map(|i| item("b", i, &tx)).collect());

        let lote = pendentes.tirar_lote(4);
        let de_a = lote.iter().filter(|i| i.texto == "a").count();
        let de_b = lote.iter().filter(|i| i.texto == "b").count();
        assert_eq!(lote.len(), 4);
        assert_eq!((de_a, de_b), (2, 2), "rodízio dá metade do lote a cada org");
        assert_eq!(pendentes.total, 4);
    }

    #[test]
    fn pedido_que_saiu_nao_entra_no_lote() {
        let (tx, rx) = mpsc::unbounded_channel();
        let org = OrgId::generate();
        let mut pendentes = Pendentes::default();
        pendentes.adicionar(org, vec![item("sai", 0, &tx), item("fica", 1, &tx)]);
        drop(rx);
        // O `tx` de cima ainda existe, mas o receptor saiu: os itens ficam com canal fechado.
        let lote = pendentes.tirar_lote(8);
        assert!(lote.is_empty(), "itens de pedido fechado são descartados");
        assert_eq!(pendentes.total, 0);
    }
}
