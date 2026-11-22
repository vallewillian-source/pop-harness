//! Limite de chamadas simultâneas ao provedor, por org e no total da instância.
//!
//! Uma chamada ocupa uma vaga da org e uma vaga global, e segura as duas até o stream terminar
//! (ou ser cancelado/solto). Org que satura a própria vaga não ocupa a vaga global de quem está
//! esperando, então não atrasa outras orgs além da fila global.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use poph_types::OrgId;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::erro::ErroLlm;

/// Vagas de chamada ao provedor: uma global e uma por org, criada na primeira chamada da org.
#[derive(Debug, Clone)]
pub struct LimitadorLlm {
    global: Arc<Semaphore>,
    por_org: usize,
    orgs: Arc<Mutex<HashMap<OrgId, Arc<Semaphore>>>>,
}

/// Vagas seguradas por uma chamada em andamento. Soltar devolve as vagas.
#[derive(Debug)]
pub struct PermissaoLlm {
    _org: OwnedSemaphorePermit,
    _global: OwnedSemaphorePermit,
}

impl LimitadorLlm {
    /// `simultaneos_global`: chamadas no total da instância. `simultaneos_por_org`: por org.
    pub fn new(simultaneos_global: usize, simultaneos_por_org: usize) -> Result<Self, ErroLlm> {
        if simultaneos_global == 0 || simultaneos_por_org == 0 {
            return Err(ErroLlm::Configuracao(
                "limites de simultaneidade devem ser maiores que zero".to_owned(),
            ));
        }
        Ok(Self {
            global: Arc::new(Semaphore::new(simultaneos_global)),
            por_org: simultaneos_por_org,
            orgs: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Espera uma vaga da org e uma vaga global. Quem chama pode cancelar a espera soltando o future.
    pub async fn adquirir(&self, org: OrgId) -> Result<PermissaoLlm, ErroLlm> {
        let da_org = self.semaforo_da_org(org);
        let org = da_org
            .acquire_owned()
            .await
            .map_err(|_| ErroLlm::Configuracao("limitador fechado".to_owned()))?;
        let global = Arc::clone(&self.global)
            .acquire_owned()
            .await
            .map_err(|_| ErroLlm::Configuracao("limitador fechado".to_owned()))?;
        Ok(PermissaoLlm {
            _org: org,
            _global: global,
        })
    }

    /// Vagas livres agora na org (para teste e para métrica futura).
    pub fn livres_da_org(&self, org: OrgId) -> usize {
        self.semaforo_da_org(org).available_permits()
    }

    /// Vagas livres agora no total da instância.
    pub fn livres_globais(&self) -> usize {
        self.global.available_permits()
    }

    fn semaforo_da_org(&self, org: OrgId) -> Arc<Semaphore> {
        let mut orgs = match self.orgs.lock() {
            Ok(guarda) => guarda,
            Err(veneno) => veneno.into_inner(),
        };
        Arc::clone(
            orgs.entry(org)
                .or_insert_with(|| Arc::new(Semaphore::new(self.por_org))),
        )
    }
}

#[cfg(test)]
mod testes {
    use std::time::Duration;

    use super::*;

    #[test]
    fn zero_vagas_e_configuracao_invalida() {
        assert!(LimitadorLlm::new(0, 1).is_err());
        assert!(LimitadorLlm::new(1, 0).is_err());
    }

    #[tokio::test]
    async fn org_saturada_nao_atrasa_outra_org() {
        let limitador = LimitadorLlm::new(10, 1).expect("limites válidos");
        let a = OrgId::generate();
        let b = OrgId::generate();

        let _segura_a = limitador.adquirir(a).await.expect("primeira de A");
        assert_eq!(limitador.livres_da_org(a), 0);

        // A segunda chamada de A fica esperando; B passa na hora.
        let espera_a = tokio::time::timeout(Duration::from_millis(50), limitador.adquirir(a));
        assert!(espera_a.await.is_err(), "A deveria esperar a própria vaga");

        let b_ok = tokio::time::timeout(Duration::from_millis(200), limitador.adquirir(b)).await;
        assert!(b_ok.is_ok(), "B não pode esperar por causa de A");
    }

    #[tokio::test]
    async fn limite_global_vale_entre_orgs_e_solta_ao_terminar() {
        let limitador = LimitadorLlm::new(2, 5).expect("limites válidos");
        let p1 = limitador.adquirir(OrgId::generate()).await.expect("1");
        let _p2 = limitador.adquirir(OrgId::generate()).await.expect("2");
        assert_eq!(limitador.livres_globais(), 0);

        let terceira = tokio::time::timeout(
            Duration::from_millis(50),
            limitador.adquirir(OrgId::generate()),
        );
        assert!(
            terceira.await.is_err(),
            "o global de 2 não pode deixar passar a 3ª"
        );

        drop(p1);
        assert_eq!(limitador.livres_globais(), 1);
    }
}
