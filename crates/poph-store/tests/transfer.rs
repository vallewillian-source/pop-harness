use std::path::PathBuf;

use poph_store::{
    ConversationStatus, NewAgent, NewConversation, NewDepartment, NewOrg, Page, Store, StoreError,
};
use poph_types::{AgentId, ConversationId, DomainError, OrgId};

struct Scenario {
    folder: PathBuf,
    store: Store,
    org: OrgId,
    other_org: OrgId,
    agent: AgentId,
}

impl Scenario {
    fn new() -> Self {
        let folder = std::env::temp_dir().join(format!(
            "poph-transferencia-store-{}-{}",
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
        Self {
            folder,
            store,
            org,
            other_org,
            agent,
        }
    }

    fn conversation(&self) -> ConversationId {
        self.store
            .conversations()
            .create(
                self.org,
                NewConversation {
                    agent_id: self.agent,
                    integration_id: None,
                    canal: "cli".to_owned(),
                    peer_externo: None,
                    target_department: None,
                },
            )
            .expect("abre a conversa")
            .id
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
            slug: slug.to_owned(),
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
                slug: slug.to_owned(),
                ..NewAgent::default()
            },
        )
        .expect("cria o agente")
        .id
}

#[test]
fn transfer_changes_situation_and_audits() {
    let scenario = Scenario::new();
    let conversation = scenario.conversation();
    let before = scenario
        .store
        .conversations()
        .get(scenario.org, conversation)
        .expect("lê a conversa");
    assert_eq!(before.status, ConversationStatus::Open);

    let later = scenario
        .store
        .conversations()
        .transfer(
            scenario.org,
            conversation,
            None,
            Some("turno".to_owned()),
            r#"{"gate":"g1"}"#.to_owned(),
        )
        .expect("transfere");

    assert_eq!(later.status, ConversationStatus::Transferred);
    assert_eq!(later.version, before.version + 1);
    assert!(later.updated_at >= before.updated_at);

    let audit = scenario
        .store
        .audit()
        .list(scenario.org, Page::with_limit(10))
        .expect("lê a auditoria");
    let registro = audit
        .iter()
        .find(|registro| registro.acao == "transfer")
        .expect("a transferência é auditada");
    assert_eq!(registro.ator_tipo, "system");
    assert_eq!(registro.actor_id.as_deref(), Some("turno"));
    assert_eq!(registro.entity, "conversation");
    assert_eq!(
        registro.entity_id.as_deref(),
        Some(conversation.to_string().as_str())
    );
    assert_eq!(registro.detail_json, r#"{"gate":"g1"}"#);
}

#[test]
fn transfer_keeps_department() {
    let scenario = Scenario::new();
    let conversation = scenario.conversation();
    let departamento = scenario
        .store
        .departments()
        .create(
            scenario.org,
            NewDepartment {
                name: "Atendimento humano".to_owned(),
                description: String::new(),
            },
        )
        .expect("cria o departamento")
        .id;

    let later = scenario
        .store
        .conversations()
        .transfer(
            scenario.org,
            conversation,
            Some(departamento),
            None,
            "{}".to_owned(),
        )
        .expect("transfere");
    assert_eq!(later.target_department, Some(departamento));

    let from_new = scenario
        .store
        .conversations()
        .transfer(scenario.org, conversation, None, None, "{}".to_owned())
        .expect("transfere de novo");
    assert_eq!(from_new.target_department, Some(departamento));
}

#[test]
fn does_not_transfer_conversation_of_other_org() {
    let scenario = Scenario::new();
    let conversation = scenario.conversation();

    let error = scenario
        .store
        .conversations()
        .transfer(
            scenario.other_org,
            conversation,
            None,
            Some("turno".to_owned()),
            "{}".to_owned(),
        )
        .expect_err("não pode transferir conversa de outra org");
    assert!(matches!(
        error,
        StoreError::Domain(DomainError::NotFound { .. })
    ));

    let stored = scenario
        .store
        .conversations()
        .get(scenario.org, conversation)
        .expect("lê a conversa");
    assert_eq!(stored.status, ConversationStatus::Open);
    assert!(
        scenario
            .store
            .audit()
            .list(scenario.other_org, Page::with_limit(10))
            .expect("lê a auditoria")
            .is_empty()
    );
}

#[test]
fn department_from_other_org_is_refused() {
    let scenario = Scenario::new();
    let conversation = scenario.conversation();
    let foreign = scenario
        .store
        .departments()
        .create(
            scenario.other_org,
            NewDepartment {
                name: "Departamento da outra org".to_owned(),
                description: String::new(),
            },
        )
        .expect("cria o departamento")
        .id;

    let error = scenario
        .store
        .conversations()
        .transfer(
            scenario.org,
            conversation,
            Some(foreign),
            None,
            "{}".to_owned(),
        )
        .expect_err("departamento de outra org é recusado");
    assert!(matches!(
        error,
        StoreError::Domain(DomainError::NotFound { .. })
    ));
    let stored = scenario
        .store
        .conversations()
        .get(scenario.org, conversation)
        .expect("lê a conversa");
    assert_eq!(stored.status, ConversationStatus::Open);
}

#[test]
fn administrative_transfer_refuses_closed_conversation() {
    let scenario = Scenario::new();
    let conversation = scenario.conversation();
    scenario
        .store
        .conversations()
        .close_by_agent(scenario.org, conversation, scenario.agent, "fim".to_owned())
        .expect("fecha pelo agente");

    let error = scenario
        .store
        .conversations()
        .transfer_audited(
            scenario.org,
            conversation,
            None,
            Some("admin".to_owned()),
            "{}".to_owned(),
        )
        .expect_err("conversa encerrada não transfere");
    assert!(matches!(
        error,
        StoreError::Domain(DomainError::ConversationClosed { .. })
    ));

    let stored = scenario
        .store
        .conversations()
        .get(scenario.org, conversation)
        .expect("lê a conversa");
    assert_eq!(stored.status, ConversationStatus::Closed);
    assert!(
        !scenario
            .store
            .audit()
            .list(scenario.org, Page::with_limit(10))
            .expect("lê a auditoria")
            .iter()
            .any(|registro| registro.acao == "transfer"),
        "a recusa não escreve auditoria de transferência"
    );
}

#[test]
fn administrative_transfer_marks_transferred_and_audits_reason() {
    let scenario = Scenario::new();
    let conversation = scenario.conversation();
    let later = scenario
        .store
        .conversations()
        .transfer_audited(
            scenario.org,
            conversation,
            None,
            Some("admin".to_owned()),
            r#"{"motivo":"rotina"}"#.to_owned(),
        )
        .expect("transfere");
    assert_eq!(later.status, ConversationStatus::Transferred);

    let audit = scenario
        .store
        .audit()
        .list(scenario.org, Page::with_limit(10))
        .expect("lê a auditoria");
    let registro = audit
        .iter()
        .find(|registro| registro.acao == "transfer")
        .expect("a transferência é auditada");
    assert_eq!(registro.actor_id.as_deref(), Some("admin"));
    assert_eq!(registro.detail_json, r#"{"motivo":"rotina"}"#);
}
