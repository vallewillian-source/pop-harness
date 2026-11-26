use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_store::{AgentChange, NewAgent, NewOrg, Org, Store, StoreError};
use poph_types::DomainError;

struct Scenario {
    folder: PathBuf,
    store: Option<Store>,
}

impl Scenario {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let folder =
            std::env::temp_dir().join(format!("poph-erro-{label}-{}-{n}", std::process::id()));
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
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = self.store.take();
        let _ = fs::remove_dir_all(&self.folder);
    }
}

fn create_org(store: &Store) -> Org {
    store
        .orgs()
        .create(NewOrg {
            name: "Org de erro".to_owned(),
            slug: "org-erro".to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org")
}

fn new_agent(slug: &str) -> NewAgent {
    NewAgent {
        name: format!("Agente {slug}"),
        slug: slug.to_owned(),
        ..NewAgent::default()
    }
}

fn validation(error: StoreError) -> (&'static str, String) {
    match error {
        StoreError::Domain(DomainError::Validation { field, reason }) => (field, reason),
        another => panic!("esperava validação de campo, veio {another:?}"),
    }
}

#[test]
fn repeated_slug_in_agent_remains_duplicate() {
    let scenario = Scenario::new("duplicado");
    let store = scenario.store();
    let org = create_org(store);
    store
        .agents()
        .create(org.id, new_agent("beatriz"))
        .expect("cria o primeiro agente");

    let (campo, motivo) = validation(
        store
            .agents()
            .create(org.id, new_agent("beatriz"))
            .expect_err("slug repetido devia ser recusado"),
    );
    assert_eq!(campo, "slug");
    assert_eq!(motivo, "já existe um registro com esse valor nesta org");
}

#[test]
fn out_of_range_context_in_agent_patch_names_right_field() {
    let scenario = Scenario::new("faixa-contexto");
    let store = scenario.store();
    let org = create_org(store);
    let agent = store
        .agents()
        .create(org.id, new_agent("beatriz"))
        .expect("cria o agente");

    let (campo, motivo) = validation(
        store
            .agents()
            .update(
                org.id,
                agent.id,
                agent.version,
                AgentChange {
                    max_context_tokens: Some(1),
                    ..AgentChange::default()
                },
            )
            .expect_err("contexto abaixo da faixa devia ser recusado"),
    );
    assert_eq!(campo, "max_context_tokens");
    assert_eq!(
        motivo,
        "valor fora da faixa ou da lista permitida para este campo"
    );
}

#[test]
fn out_of_range_tools_limit_names_api_field() {
    let scenario = Scenario::new("faixa-ferramentas");
    let store = scenario.store();
    let org = create_org(store);
    let agent = store
        .agents()
        .create(org.id, new_agent("beatriz"))
        .expect("cria o agente");

    let (campo, _) = validation(
        store
            .agents()
            .update(
                org.id,
                agent.id,
                agent.version,
                AgentChange {
                    max_tools_per_message: Some(101),
                    ..AgentChange::default()
                },
            )
            .expect_err("101 chamadas devia ser recusado"),
    );
    assert_eq!(campo, "max_tool_calls");
}

#[test]
fn rejected_range_writes_nothing() {
    let scenario = Scenario::new("nada-gravado");
    let store = scenario.store();
    let org = create_org(store);
    let agent = store
        .agents()
        .create(org.id, new_agent("beatriz"))
        .expect("cria o agente");

    let _ = store.agents().update(
        org.id,
        agent.id,
        agent.version,
        AgentChange {
            max_context_tokens: Some(1),
            ..AgentChange::default()
        },
    );
    let later = store.agents().get(org.id, agent.id).expect("lê o agente");
    assert_eq!(later.max_context_tokens, agent.max_context_tokens);
    assert_eq!(later.version, agent.version);
}
