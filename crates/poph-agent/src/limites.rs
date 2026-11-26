//! Limites de simultaneidade e de taxa por org, por agente e por provedor (PRD §6.4, decisão D-07).
//!
//! Regra de ouro: **org saturada não atrasa outra org**. Cada org tem o seu semáforo; o semáforo
//! global existe para o teto da instância (200 turnos, D-07), não para distribuir vaga. Do mesmo
//! jeito, o teto por agente é separado do teto por org, e o do provedor (`llm.simultaneos`) fica no
//! nível das chamadas ao modelo.
//!
//! A vaga devolvida por [`Limitador::adquirir_turno`] é um empréstimo: solta quando `Vaga` sai de
//! escopo (fim do turno). Não é preciso chamar nada.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use poph_types::{AgentId, LimitsConfig, LlmConfig, OrgId};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::erro::{ErroAgente, Resultado};
use crate::metricas::MetricasMemoria;

/// Números que o limitador carrega. Vêm de [`LimitesConfig`] e dos parâmetros do LLM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpcoesLimites {
    /// Turnos rodando ao mesmo tempo na instância.
    pub turnos_global: u32,
    /// Turnos rodando ao mesmo tempo por org.
    pub turnos_por_org: u32,
    /// Turnos rodando ao mesmo tempo por agente (0 = sem teto próprio).
    pub turnos_por_agente: u32,
    /// Chamadas simultâneas por provedor de LLM.
    pub chamadas_por_provedor: u32,
    pub tetos_por_provedor: BTreeMap<String, u32>,
    /// Mensagens aceitas por minuto em cada org (0 = sem teto).
    pub taxa_por_org_por_minuto: u32,
}

impl Default for OpcoesLimites {
    fn default() -> Self {
        let limites = LimitsConfig::default();
        Self::dos_limites(&limites)
    }
}

impl OpcoesLimites {
    /// Deriva do bloco `limites` da configuração; o teto por agente é opcional.
    pub fn dos_limites(limites: &LimitsConfig) -> Self {
        Self {
            turnos_global: limites.concurrent_turns.max(1),
            turnos_por_org: limites
                .concurrent_turns_per_org
                .clamp(1, limites.concurrent_turns.max(1)),
            turnos_por_agente: 0,
            chamadas_por_provedor: 32,
            tetos_por_provedor: BTreeMap::new(),
            taxa_por_org_por_minuto: limites.rate_per_org_per_minute,
        }
    }

    /// Lê os dois blocos da configuração: `limites` (turnos) e `llm` (chamadas ao provedor).
    pub fn da_config(limites: &LimitsConfig, llm: &LlmConfig) -> Self {
        Self::dos_limites(limites).com_provedor(llm.concurrent_calls)
    }

    /// Define o teto de chamadas simultâneas de um provedor (vem de `llm.simultaneos`).
    pub fn com_provedor(mut self, simultaneas: u32) -> Self {
        self.chamadas_por_provedor = simultaneas.max(1);
        self
    }

    pub fn com_tetos_por_provedor(mut self, tetos: BTreeMap<String, u32>) -> Self {
        self.tetos_por_provedor = tetos;
        self
    }

    /// Define o teto por agente (0 = sem teto próprio).
    pub fn com_agente(mut self, simultaneos: u32) -> Self {
        self.turnos_por_agente = simultaneos;
        self
    }
}

/// Empréstimo de uma vaga de turno. Solta as três ao sair de escopo.
#[derive(Debug)]
pub struct Vaga {
    _global: OwnedSemaphorePermit,
    _org: OwnedSemaphorePermit,
    _agente: Option<OwnedSemaphorePermit>,
}

/// Empréstimo de uma vaga de chamada ao provedor.
#[derive(Debug)]
pub struct VagaProvedor {
    _permit: OwnedSemaphorePermit,
}

/// Limitador de simultaneidade e de taxa.
pub struct Limitador {
    opcoes: OpcoesLimites,
    global: Arc<Semaphore>,
    por_org: Mutex<HashMap<OrgId, Arc<Semaphore>>>,
    por_agente: Mutex<HashMap<(OrgId, AgentId), Arc<Semaphore>>>,
    por_provedor: Mutex<HashMap<String, Arc<Semaphore>>>,
    taxas: Mutex<HashMap<OrgId, Balde>>,
    metricas: MetricasMemoria,
}

#[derive(Debug)]
struct Balde {
    tokens: f64,
    ultimo: Instant,
}

impl Limitador {
    /// Cria o limitador com os números dados.
    pub fn novo(opcoes: OpcoesLimites, metricas: MetricasMemoria) -> Self {
        Self {
            global: Arc::new(Semaphore::new(opcoes.turnos_global as usize)),
            por_org: Mutex::new(HashMap::new()),
            por_agente: Mutex::new(HashMap::new()),
            por_provedor: Mutex::new(HashMap::new()),
            taxas: Mutex::new(HashMap::new()),
            opcoes,
            metricas,
        }
    }

    /// Configuração em uso.
    pub fn opcoes(&self) -> &OpcoesLimites {
        &self.opcoes
    }

    /// Pede uma vaga de turno e espera até `espera`. Os semáforos são adquiridos do mais amplo para
    /// o mais estreito; se um deles estourar o tempo, os já obtidos são devolvidos pelo `Drop`.
    pub async fn adquirir_turno(
        &self,
        org: OrgId,
        agente: AgentId,
        espera: Duration,
    ) -> Resultado<Vaga> {
        let inicio = Instant::now();
        let global = self.adquirir(&self.global, "instância", espera).await?;
        let org_semaforo = self.semaforo_da_org(org);
        let por_org = self.adquirir(&org_semaforo, "org", espera).await?;
        let por_agente = if self.opcoes.turnos_por_agente == 0 {
            None
        } else {
            let semaforo = self.semaforo_do_agente(org, agente);
            Some(self.adquirir(&semaforo, "agente", espera).await?)
        };
        self.metricas
            .espera_vaga_segundos
            .observe(inicio.elapsed().as_secs_f64());
        Ok(Vaga {
            _global: global,
            _org: por_org,
            _agente: por_agente,
        })
    }

    /// Pede uma vaga de chamada ao provedor (o teto de `llm.simultaneos`).
    pub async fn adquirir_provedor(
        &self,
        provedor: &str,
        espera: Duration,
    ) -> Resultado<VagaProvedor> {
        let semaforo = self.semaforo_do_provedor(provedor);
        let permit = match tokio::time::timeout(espera, semaforo.acquire_owned()).await {
            Ok(Ok(permit)) => permit,
            Ok(Err(_)) => {
                return Err(ErroAgente::Interno(format!(
                    "semáforo do provedor `{provedor}` fechado inesperadamente"
                )));
            }
            Err(_) => {
                return Err(ErroAgente::SemVagaProvedor {
                    provedor: provedor.to_owned(),
                    espera_ms: espera.as_millis() as u64,
                });
            }
        };
        Ok(VagaProvedor { _permit: permit })
    }

    /// Consome uma mensagem da cota por minuto da org. `Ok(())` quando havia cota.
    pub fn consumir_cota(&self, org: OrgId) -> Resultado<()> {
        let limite = self.opcoes.taxa_por_org_por_minuto;
        if limite == 0 {
            return Ok(());
        }
        let mut taxas = match self.taxas.lock() {
            Ok(guarda) => guarda,
            Err(veneno) => veneno.into_inner(),
        };
        let agora = Instant::now();
        let balde = taxas.entry(org).or_insert(Balde {
            tokens: f64::from(limite),
            ultimo: agora,
        });
        let passado = agora.duration_since(balde.ultimo).as_secs_f64();
        balde.tokens = (balde.tokens + passado * (f64::from(limite) / 60.0)).min(f64::from(limite));
        balde.ultimo = agora;
        if balde.tokens >= 1.0 {
            balde.tokens -= 1.0;
            return Ok(());
        }
        let faltando = 1.0 - balde.tokens;
        let esperar_ms = (faltando * 60.0 / f64::from(limite) * 1000.0).ceil() as u64;
        Err(ErroAgente::CotaEsgotada { limite, esperar_ms })
    }

    async fn adquirir(
        &self,
        semaforo: &Arc<Semaphore>,
        escopo: &str,
        espera: Duration,
    ) -> Resultado<OwnedSemaphorePermit> {
        match tokio::time::timeout(espera, Arc::clone(semaforo).acquire_owned()).await {
            Ok(Ok(permit)) => Ok(permit),
            Ok(Err(_)) => Err(ErroAgente::Interno(format!(
                "semáforo de {escopo} fechado inesperadamente"
            ))),
            Err(_) => Err(ErroAgente::SemVaga {
                escopo: escopo.to_owned(),
                espera_ms: espera.as_millis() as u64,
            }),
        }
    }

    fn semaforo_da_org(&self, org: OrgId) -> Arc<Semaphore> {
        let mut mapa = self.por_org.lock().unwrap_or_else(|erro| erro.into_inner());
        Arc::clone(
            mapa.entry(org)
                .or_insert_with(|| Arc::new(Semaphore::new(self.opcoes.turnos_por_org as usize))),
        )
    }

    fn semaforo_do_agente(&self, org: OrgId, agente: AgentId) -> Arc<Semaphore> {
        let maximo = self.opcoes.turnos_por_agente as usize;
        let mut mapa = self
            .por_agente
            .lock()
            .unwrap_or_else(|erro| erro.into_inner());
        Arc::clone(
            mapa.entry((org, agente))
                .or_insert_with(|| Arc::new(Semaphore::new(maximo))),
        )
    }

    fn semaforo_do_provedor(&self, provedor: &str) -> Arc<Semaphore> {
        let maximo = self
            .opcoes
            .tetos_por_provedor
            .get(provedor)
            .copied()
            .unwrap_or(self.opcoes.chamadas_por_provedor)
            .max(1) as usize;
        let mut mapa = self
            .por_provedor
            .lock()
            .unwrap_or_else(|erro| erro.into_inner());
        Arc::clone(
            mapa.entry(provedor.to_owned())
                .or_insert_with(|| Arc::new(Semaphore::new(maximo))),
        )
    }
}
