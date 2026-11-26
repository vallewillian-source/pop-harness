use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use poph_types::OrgId;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::error::LlmError;

#[derive(Debug, Clone)]
pub struct LlmLimiter {
    global: Arc<Semaphore>,
    per_org: usize,
    orgs: Arc<Mutex<HashMap<OrgId, Arc<Semaphore>>>>,
}

#[derive(Debug)]
pub struct LlmPermit {
    _org: OwnedSemaphorePermit,
    _global: OwnedSemaphorePermit,
}

impl LlmLimiter {
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

    pub fn available_for_org(&self, org: OrgId) -> usize {
        self.semaphore_for_org(org).available_permits()
    }

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
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn zero_slots_are_invalid_configuration() {
        assert!(LlmLimiter::new(0, 1).is_err());
        assert!(LlmLimiter::new(1, 0).is_err());
    }

    #[tokio::test]
    async fn saturated_org_does_not_delay_another_org() {
        let limiter = LlmLimiter::new(10, 1).expect("limites válidos");
        let a = OrgId::generate();
        let b = OrgId::generate();

        let _hold_a = limiter.acquire(a).await.expect("primeira de A");
        assert_eq!(limiter.available_for_org(a), 0);

        let wait_a = tokio::time::timeout(Duration::from_millis(50), limiter.acquire(a));
        assert!(wait_a.await.is_err(), "A deveria esperar a própria vaga");

        let b_ok = tokio::time::timeout(Duration::from_millis(200), limiter.acquire(b)).await;
        assert!(b_ok.is_ok(), "B não pode esperar por causa de A");
    }

    #[tokio::test]
    async fn global_limit_applies_across_orgs_and_releases_on_completion() {
        let limiter = LlmLimiter::new(2, 5).expect("limites válidos");
        let p1 = limiter.acquire(OrgId::generate()).await.expect("1");
        let _p2 = limiter.acquire(OrgId::generate()).await.expect("2");
        assert_eq!(limiter.available_globally(), 0);

        let third = tokio::time::timeout(
            Duration::from_millis(50),
            limiter.acquire(OrgId::generate()),
        );
        assert!(
            third.await.is_err(),
            "o global de 2 não pode deixar passar a 3ª"
        );

        drop(p1);
        assert_eq!(limiter.available_globally(), 1);
    }
}
