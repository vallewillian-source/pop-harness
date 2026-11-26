use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_store::{AgentChange, NewAgent, NewOrg, Org, Status, Store};
use poph_types::OrgId;

struct Scenario {
    folder: PathBuf,
    store: Option<Store>,
}

impl Scenario {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let folder =
            std::env::temp_dir().join(format!("poph-orgaos-{label}-{}-{n}", std::process::id()));
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
        self.store.as_ref().expect("store viva")
    }

    fn org(&self, slug: &str) -> Org {
        self.store()
            .orgs()
            .create(NewOrg {
                name: format!("Org {slug}"),
                slug: slug.to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
    }

    fn agent(&self, org: &Org, slug: &str) -> poph_store::Agente {
        self.store()
            .agents()
            .create(
                org.id,
                NewAgent {
                    name: format!("Agente {slug}"),
                    slug: slug.to_owned(),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
    }

    fn situation(&self, org: &Org, agent: &poph_store::Agente, status: Status) {
        let current = self
            .store()
            .agents()
            .get(org.id, agent.id)
            .expect("lê o agente");
        self.store()
            .agents()
            .update(
                org.id,
                agent.id,
                current.version,
                AgentChange {
                    status: Some(status),
                    ..AgentChange::default()
                },
            )
            .expect("muda a situação");
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        self.store = None;
        let _ = fs::remove_dir_all(&self.folder);
    }
}

#[test]
fn counts_only_active_agent_and_omits_org_without_agent() {
    let scenario = Scenario::new("contagem");
    let with_agents = scenario.org("com-agentes");
    let without_agents = scenario.org("sem-agentes");

    for slug in ["um", "dois", "tres"] {
        scenario.agent(&with_agents, slug);
    }
    let inactive = scenario.agent(&with_agents, "quatro");
    scenario.situation(&with_agents, &inactive, Status::Inactive);
    let archived = scenario.agent(&with_agents, "cinco");
    scenario.situation(&with_agents, &archived, Status::Archived);

    let count = scenario
        .store()
        .agents()
        .count_active_by_org(&[with_agents.id, without_agents.id])
        .expect("conta os ativos");

    assert_eq!(
        count.get(&with_agents.id).copied(),
        Some(3),
        "inativo e arquivado não podem entrar na conta: {count:?}"
    );
    assert_eq!(
        count.get(&without_agents.id),
        None,
        "org sem agente ativo não aparece no mapa (quem chama usa 0): {count:?}"
    );
    assert_eq!(count.len(), 1);
}

#[test]
fn page_count_is_a_single_query() {
    let scenario = Scenario::new("uma-consulta");
    let mut ids: Vec<OrgId> = Vec::new();
    for index in 0..5 {
        let org = scenario.org(&format!("org-{index}"));
        if index % 2 == 0 {
            scenario.agent(&org, "a");
        }
        ids.push(org.id);
    }

    let before = scenario.store().reads_done();
    let count = scenario
        .store()
        .agents()
        .count_active_by_org(&ids)
        .expect("conta os ativos");
    let later = scenario.store().reads_done();

    assert_eq!(count.len(), 3, "três orgs com um agente ativo cada");
    assert_eq!(
        later - before,
        1,
        "a contagem da página inteira tem de custar uma leitura, não uma por org"
    );

    let before = scenario.store().reads_done();
    let empty = scenario
        .store()
        .agents()
        .count_active_by_org(&[])
        .expect("lista vazia");
    assert!(empty.is_empty());
    assert_eq!(scenario.store().reads_done(), before);
}

#[test]
fn counts_only_requested_orgs() {
    let scenario = Scenario::new("fechada-na-pagina");
    let requested = scenario.org("pedida");
    let other = scenario.org("outra");
    scenario.agent(&requested, "a");
    scenario.agent(&other, "b");
    scenario.agent(&other, "c");

    let count = scenario
        .store()
        .agents()
        .count_active_by_org(&[requested.id])
        .expect("conta os ativos");

    assert_eq!(count.get(&requested.id).copied(), Some(1));
    assert_eq!(
        count.get(&other.id),
        None,
        "a org fora da página não pode aparecer: {count:?}"
    );
}
