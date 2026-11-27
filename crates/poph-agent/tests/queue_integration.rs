use std::sync::Arc;

use poph_agent::queue_integration::{ConversationChannel, ResolverConversation};
use poph_store::{ConversationStatus, NewAgent, NewConversation, NewOrg, Store};
use poph_types::{AgentId, InboundEvent, IntegrationId, OrgId};

struct Scenario {
    folder: std::path::PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
}

impl Scenario {
    fn new(label: &str) -> Self {
        let folder = std::env::temp_dir().join(format!(
            "poph-fila-integracao-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("cria a pasta");
        let store = Arc::new(
            Store::open_at(&folder.join("harness.db"), &folder.join("rag.db"))
                .expect("abre o banco"),
        );
        let org = store
            .orgs()
            .create(NewOrg {
                name: format!("Org {label}"),
                slug: format!("org-{label}"),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: format!("Agente {label}"),
                    slug: format!("agente-{label}"),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
            .id;
        Self {
            folder,
            store,
            org,
            agente,
        }
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

fn evento(peer: &str) -> InboundEvent {
    InboundEvent {
        integration: IntegrationId::generate(),
        id_externo: format!("msg-{peer}"),
        peer: peer.to_owned(),
        remetente: None,
        text: "oi".to_owned(),
        attachments: Vec::new(),
        occurred_at: 1_795_140_000_000,
        metadata: serde_json::Value::Null,
    }
}

#[test]
fn contact_conversation_with_cache_by_contact() {
    let scenario = Scenario::new("cache");
    let resolver =
        ConversationChannel::new(Arc::clone(&scenario.store), scenario.agente, "eco", None);

    let first = resolver
        .resolver(scenario.org, &evento("5511911111111"))
        .expect("resolve o primeiro contato");
    let repeated = resolver
        .resolver(scenario.org, &evento("5511911111111"))
        .expect("resolve de novo");
    assert_eq!(first, repeated);
    assert_eq!(resolver.em_cache(), 1);

    let other = resolver
        .resolver(scenario.org, &evento("5511922222222"))
        .expect("resolve o segundo contato");
    assert_ne!(first, other);
    assert_eq!(resolver.em_cache(), 2);

    let conversas = scenario
        .store
        .conversations()
        .list(scenario.org, None, poph_store::Page::with_limit(10))
        .expect("lista as conversas");
    assert_eq!(conversas.len(), 2);
    assert!(
        conversas
            .iter()
            .all(|conversa| conversa.canal == "eco" && conversa.agent_id == scenario.agente)
    );
}

#[test]
fn conversation_transferred_not_served_cache() {
    let scenario = Scenario::new("transferida");
    let resolver =
        ConversationChannel::new(Arc::clone(&scenario.store), scenario.agente, "eco", None);
    let peer = "5511933333333";

    let conversa = resolver
        .resolver(scenario.org, &evento(peer))
        .expect("resolve o contato");
    scenario
        .store
        .conversations()
        .change_status(
            scenario.org,
            conversa,
            1,
            ConversationStatus::Transferred,
            None,
        )
        .expect("transfere a conversa");

    let new = scenario
        .store
        .conversations()
        .create(
            scenario.org,
            NewConversation {
                agent_id: scenario.agente,
                integration_id: None,
                canal: "eco".to_owned(),
                peer_externo: Some(peer.to_owned()),
                target_department: None,
            },
        )
        .expect("abre a conversa nova");
    let chosen = resolver
        .resolver(scenario.org, &evento(peer))
        .expect("resolve de novo");
    assert_eq!(chosen, new.id);
}
