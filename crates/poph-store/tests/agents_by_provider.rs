use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_store::{AgentChange, NewAgent, NewOrg, Store, agents_by_provider_at};
use poph_types::OrgId;

struct Scenario {
    folder: PathBuf,
    store: Option<Store>,
}

impl Scenario {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let folder = std::env::temp_dir().join(format!(
            "poph-agentes-provedor-{label}-{}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("cria a pasta do teste");
        let store = Store::open_at(&folder.join("harness.db"), &folder.join("rag.db"))
            .expect("abre o banco");
        Self {
            folder,
            store: Some(store),
        }
    }

    fn store(&self) -> &Store {
        self.store.as_ref().expect("banco aberto")
    }

    fn org(&self) -> OrgId {
        self.store()
            .orgs()
            .create(NewOrg {
                name: "Org provedores".to_owned(),
                slug: format!("org-provedores-{}", std::process::id()),
                config_json: "{}".to_owned(),
            })
            .expect("cria a org")
            .id
    }

    fn agent(&self, org: OrgId, nome: &str) -> poph_types::AgentId {
        self.store()
            .agents()
            .create(
                org,
                NewAgent {
                    name: nome.to_owned(),
                    slug: nome.to_lowercase().replace(' ', "-"),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
            .id
    }

    fn swap_provider(&self, org: OrgId, id: poph_types::AgentId, provedor: &str) {
        let agent = self.store().agents().get(org, id).expect("lê o agente");
        self.store()
            .agents()
            .update(
                org,
                id,
                agent.version,
                AgentChange {
                    provedor: Some(provedor.to_owned()),
                    ..AgentChange::default()
                },
            )
            .expect("troca o provedor");
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        self.store = None;
        let _ = fs::remove_dir_all(&self.folder);
    }
}

#[test]
fn counts_agents_by_provider_without_creating_anything() {
    let scenario = Scenario::new("contagem");
    let org = scenario.org();
    let first_value = scenario.agent(org, "Ana");
    let second_value = scenario.agent(org, "Bruno");
    let third_value = scenario.agent(org, "Carla");
    scenario.swap_provider(org, second_value, "openai-forte");
    scenario.swap_provider(org, third_value, "openai-forte");

    let path = scenario.folder.join("harness.db");
    let count = agents_by_provider_at(&path).expect("conta os agentes");
    assert_eq!(
        count,
        vec![
            ("antigravity2api".to_owned(), 1),
            ("openai-forte".to_owned(), 2),
        ]
    );
    assert_eq!(
        scenario
            .store()
            .agents()
            .get(org, first_value)
            .expect("o agente continua lá")
            .provedor,
        "antigravity2api"
    );
}

#[test]
fn missing_database_is_error_and_creates_no_file() {
    let folder = std::env::temp_dir().join(format!(
        "poph-agentes-provedor-sem-banco-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&folder);
    fs::create_dir_all(&folder).expect("cria a pasta do teste");
    let path = folder.join("harness.db");
    assert!(agents_by_provider_at(&path).is_err());
    assert!(!path.exists(), "não pode criar o banco");
    let _ = fs::remove_dir_all(&folder);
}
