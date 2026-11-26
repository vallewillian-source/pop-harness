use poph_store::{Library, NewAgent, NewLibraryFile, NewOrg, Page, Store, StoreError};
use poph_types::{AgentId, FileId, OrgId};

struct Scenario {
    folder: std::path::PathBuf,
    store: Store,
    org: OrgId,
    other_org: OrgId,
    agent: AgentId,
    other_agent: AgentId,
}

impl Scenario {
    fn new() -> Self {
        let folder = std::env::temp_dir().join(format!(
            "poph-biblioteca-store-{}-{}",
            std::process::id(),
            AgentId::generate()
        ));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("cria a pasta");
        let store = Store::open_at(&folder.join("harness.db"), &folder.join("rag.db"))
            .expect("abre o banco");
        let org = create_org(&store, "org-a");
        let other_org = create_org(&store, "org-b");
        let agent = create_agent(&store, org, "bia");
        let other_agent = create_agent(&store, org, "caio");
        Self {
            folder,
            store,
            org,
            other_org,
            agent,
            other_agent,
        }
    }

    fn library(&self) -> Library<'_> {
        self.store.library()
    }

    fn new_file(&self, agent: AgentId, nome: &str) -> NewLibraryFile {
        NewLibraryFile {
            agent_id: agent,
            name: nome.to_owned(),
            description: "descrição".to_owned(),
            mime: "application/pdf".to_owned(),
            size_bytes: 1_024,
            sha256: "a".repeat(64),
            blob_path: format!("{}/{nome}", self.org),
        }
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

fn create_org(store: &Store, slug: &str) -> OrgId {
    store
        .orgs()
        .create(NewOrg {
            name: format!("Org {slug}"),
            slug: format!("{slug}-{}", AgentId::generate()),
            config_json: "{}".to_owned(),
        })
        .expect("cria a org")
        .id
}

fn create_agent(store: &Store, org: OrgId, slug: &str) -> AgentId {
    store
        .agents()
        .create(
            org,
            NewAgent {
                name: format!("Agente {slug}"),
                slug: format!("{slug}-{}", &AgentId::generate().to_string()[..8]),
                ..NewAgent::default()
            },
        )
        .expect("cria o agente")
        .id
}

#[test]
fn creates_lists_counts_and_deletes() {
    let scenario = Scenario::new();
    let file = scenario
        .library()
        .create(scenario.org, scenario.new_file(scenario.agent, "a.pdf"))
        .expect("cria o arquivo");
    assert_eq!(file.name, "a.pdf");
    assert_eq!(file.size_bytes, 1_024);
    assert_eq!(
        scenario
            .library()
            .count_rows(scenario.org, scenario.agent)
            .expect("conta"),
        1
    );
    let list = scenario
        .library()
        .list(scenario.org, scenario.agent, Page::default())
        .expect("lista");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, file.id);

    scenario
        .library()
        .delete(scenario.org, scenario.agent, file.id)
        .expect("exclui");
    assert_eq!(
        scenario
            .library()
            .count_rows(scenario.org, scenario.agent)
            .expect("conta"),
        0
    );
    let error = scenario
        .library()
        .get(scenario.org, scenario.agent, file.id)
        .expect_err("não existe mais");
    assert!(matches!(
        error,
        StoreError::Domain(poph_types::DomainError::NotFound { .. })
    ));
}

#[test]
fn another_agent_of_same_org_does_not_reach_file() {
    let scenario = Scenario::new();
    let file = scenario
        .library()
        .create(scenario.org, scenario.new_file(scenario.agent, "a.pdf"))
        .expect("cria");
    let error = scenario
        .library()
        .get(scenario.org, scenario.other_agent, file.id)
        .expect_err("outro agente não vê");
    assert!(matches!(
        error,
        StoreError::Domain(poph_types::DomainError::NotFound { .. })
    ));
    let error = scenario
        .library()
        .delete(scenario.org, scenario.other_agent, file.id)
        .expect_err("outro agente não apaga");
    assert!(matches!(
        error,
        StoreError::Domain(poph_types::DomainError::NotFound { .. })
    ));
    assert_eq!(
        scenario
            .library()
            .count_rows(scenario.org, scenario.agent)
            .expect("conta"),
        1
    );
}

#[test]
fn wrong_org_does_not_reach_file() {
    let scenario = Scenario::new();
    let file = scenario
        .library()
        .create(scenario.org, scenario.new_file(scenario.agent, "a.pdf"))
        .expect("cria");
    let error = scenario
        .library()
        .get(scenario.other_org, scenario.agent, file.id)
        .expect_err("org errada não vê");
    assert!(matches!(
        error,
        StoreError::Domain(poph_types::DomainError::NotFound { .. })
    ));
    assert_eq!(
        scenario
            .library()
            .count_rows(scenario.other_org, scenario.agent)
            .expect("conta"),
        0
    );
}

#[test]
fn agent_from_other_org_gets_no_file() {
    let scenario = Scenario::new();
    let error = scenario
        .library()
        .create(
            scenario.other_org,
            scenario.new_file(scenario.agent, "a.pdf"),
        )
        .expect_err("agente de outra org é recusado");
    assert!(
        matches!(
            error,
            StoreError::Domain(poph_types::DomainError::NotFound { .. })
        ),
        "{error:?}"
    );
}

#[test]
fn invalid_fields_give_validation_error() {
    let scenario = Scenario::new();
    let mut new = scenario.new_file(scenario.agent, " ");
    let error = scenario
        .library()
        .create(scenario.org, new.clone())
        .expect_err("nome vazio");
    assert!(matches!(
        error,
        StoreError::Domain(poph_types::DomainError::Validation { field: "nome", .. })
    ));

    new = scenario.new_file(scenario.agent, "a.pdf");
    new.sha256 = "xyz".to_owned();
    let error = scenario
        .library()
        .create(scenario.org, new)
        .expect_err("sha inválido");
    assert!(matches!(
        error,
        StoreError::Domain(poph_types::DomainError::Validation {
            field: "sha256",
            ..
        })
    ));

    let mut new = scenario.new_file(scenario.agent, "a.pdf");
    new.description = "d".repeat(poph_store::LIMIT_DESCRIPTION_FILE + 1);
    let error = scenario
        .library()
        .create(scenario.org, new)
        .expect_err("descrição longa");
    assert!(matches!(
        error,
        StoreError::Domain(poph_types::DomainError::Validation {
            field: "descrição",
            ..
        })
    ));

    let invalid: Result<FileId, _> = "nao-e-uuid".parse();
    assert!(invalid.is_err());
}
