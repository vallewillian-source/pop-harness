//! Limite de chamadas simultâneas ao provedor, por org e no total da instância.
//!
//! Uma chamada ocupa uma vaga da org e uma vaga global, e segura as duas até o stream terminar
//! (ou ser cancelado/solto). Org que satura a própria vaga não ocupa a vaga global de quem está
//! esperando, então não atrasa outras orgs além da fila global.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use poph_types::OrgId;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::erro::LlmError;

/// Vagas de chamada ao provedor: uma global e uma por org, criada na primeira chamada da org.
#[derive(Debug, Clone)]
pub struct LlmLimiter {
    global: Arc<Semaphore>,
    per_org: usize,
    orgs: Arc<Mutex<HashMap<OrgId, Arc<Semaphore>>>>,
}

/// Vagas seguradas por uma chamada em andamento. Soltar devolve as vagas.
#[derive(Debug)]
pub struct LlmPermit {
    _org: OwnedSemaphorePermit,
    _global: OwnedSemaphorePermit,
}

impl LlmLimiter {
    /// `simultaneos_global`: chamadas no total da instância. `simultaneos_por_org`: por org.
    pub fn new(global_concurrency: usize, org_concurrency: usize) -> Result<Self, LlmError> {
        if global_concurrency == 0 || org_concurrency == 0 {
            return Err(LlmError::Configuracao(
                "limites de simultaneidade devem ser maiores que zero".to_owned(),
            ));
        }
        Ok(Self {
            global: Arc::new(Semaphore::new(global_concurrency)),
            per_org: org_concurrency,
            orgs: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Espera uma vaga da org e uma vaga global. Quem chama pode cancelar a espera soltando o future.
    pub async fn acquire(&self, org: OrgId) -> Result<LlmPermit, LlmError> {
        let org_semaphore = self.semaphore_for_org(org);
        let org = org_semaphore
            .acquire_owned()
            .await
            .map_err(|_| LlmError::Configuracao("limitador fechado".to_owned()))?;
        let global = Arc::clone(&self.global)
            .acquire_owned()
            .await
            .map_err(|_| LlmError::Configuracao("limitador fechado".to_owned()))?;
        Ok(LlmPermit {
            _org: org,
            _global: global,
        })
    }

    /// Vagas livres agora na org (para teste e para métrica futura).
    pub fn available_for_org(&self, org: OrgId) -> usize {
        self.semaphore_for_org(org).available_permits()
    }

    /// Vagas livres agora no total da instância.
    pub fn available_globally(&self) -> usize {
        self.global.available_permits()
    }

    fn semaphore_for_org(&self, org: OrgId) -> Arc<Semaphore> {
        let mut orgs = match self.orgs.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        Arc::clone(
            orgs.entry(org)
                .or_insert_with(|| Arc::new(Semaphore::new(self.per_org))),
        )
    }
}

#[cfg(test)]
mod testes {
    use std::time::Duration;

    use super::*;

    #[test]
    fn zero_slots_are_invalid_configuration() {
        assert!(LlmLimiter::new(0, 1).is_err());
        assert!(LlmLimiter::new(1, 0).is_err());
    }

    #[tokio::test]
    async fn saturated_org_does_not_delay_another_org() {
        let limitador = LlmLimiter::new(10, 1).expect("limites válidos");
        let a = OrgId::generate();
        let b = OrgId::generate();

        let _segura_a = limitador.acquire(a).await.expect("primeira de A");
        assert_eq!(limitador.available_for_org(a), 0);

        // A segunda chamada de A fica esperando; B passa na hora.
        let espera_a = tokio::time::timeout(Duration::from_millis(50), limitador.acquire(a));
        assert!(espera_a.await.is_err(), "A deveria esperar a própria vaga");

        let b_ok = tokio::time::timeout(Duration::from_millis(200), limitador.acquire(b)).await;
        assert!(b_ok.is_ok(), "B não pode esperar por causa de A");
    }

    #[tokio::test]
    async fn global_limit_applies_across_orgs_and_releases_on_completion() {
        let limitador = LlmLimiter::new(2, 5).expect("limites válidos");
        let p1 = limitador.acquire(OrgId::generate()).await.expect("1");
        let _p2 = limitador.acquire(OrgId::generate()).await.expect("2");
        assert_eq!(limitador.available_globally(), 0);

        let terceira = tokio::time::timeout(
            Duration::from_millis(50),
            limitador.acquire(OrgId::generate()),
        );
        assert!(
            terceira.await.is_err(),
            "o global de 2 não pode deixar passar a 3ª"
        );

        drop(p1);
        assert_eq!(limitador.available_globally(), 1);
    }
}
