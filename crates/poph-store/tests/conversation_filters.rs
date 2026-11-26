use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use poph_store::{
    Agente, Conversa, ConversationStatus, FilterConversations, NewAgent, NewConversation, NewOrg,
    OrderConversations, Org, Page, Store,
};
use poph_types::{ConversationId, OrgId};

struct Scenario {
    folder: PathBuf,
    store: Arc<Store>,
}

impl Scenario {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let folder = std::env::temp_dir().join(format!(
            "poph-filtros-conversas-{label}-{}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("cria a pasta do teste");
        let clock = Arc::new(AtomicI64::new(1_000));
        let now = Arc::clone(&clock);
        let store = Store::open_at(&folder.join("harness.db"), &folder.join("rag.db"))
            .expect("abre o banco")
            .with_clock(Arc::new(move || now.fetch_add(10, Ordering::SeqCst)));
        Self {
            folder,
            store: Arc::new(store),
        }
    }

    fn org(&self, slug: &str) -> Org {
        self.store
            .orgs()
            .create(NewOrg {
                name: format!("Org {slug}"),
                slug: slug.to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
    }

    fn agent(&self, org: &Org, slug: &str) -> Agente {
        self.store
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

    fn conversation(&self, org: &Org, agent: &Agente, peer: Option<&str>) -> Conversa {
        self.store
            .conversations()
            .create(
                org.id,
                NewConversation {
                    agent_id: agent.id,
                    integration_id: None,
                    canal: "whatsapp".to_owned(),
                    peer_externo: peer.map(str::to_owned),
                    target_department: None,
                },
            )
            .expect("cria a conversa")
    }

    fn list(&self, org: OrgId, filter: &FilterConversations) -> Vec<ConversationId> {
        self.store
            .conversations()
            .list_filter(org, filter, Page::with_limit(200))
            .expect("lista com filtro")
            .into_iter()
            .map(|c| c.id)
            .collect()
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.folder);
    }
}

fn filter_peer(peer: &str) -> FilterConversations {
    FilterConversations {
        external_peer: Some(peer.to_owned()),
        status: None,
        ..Default::default()
    }
}

#[test]
fn customer_filter_finds_only_its_conversations_in_org() {
    let c = Scenario::new("peer");
    let a = c.org("a");
    let b = c.org("b");
    let agent_a = c.agent(&a, "a");
    let agent_b = c.agent(&b, "b");

    let c1 = c.conversation(&a, &agent_a, Some("5511"));
    let c2 = c.conversation(&a, &agent_a, Some("5511"));
    let _c3 = c.conversation(&a, &agent_a, Some("5522"));
    let d1 = c.conversation(&b, &agent_b, Some("5511"));

    let from_a = c.list(a.id, &filter_peer("5511"));
    assert_eq!(
        from_a,
        vec![c1.id, c2.id],
        "só as duas conversas do cliente, na org A"
    );

    let from_b = c.list(b.id, &filter_peer("5511"));
    assert_eq!(
        from_b,
        vec![d1.id],
        "o cliente de outra org não aparece no filtro desta org"
    );
    assert!(
        c.list(a.id, &filter_peer("9999")).is_empty(),
        "cliente sem conversa devolve lista vazia"
    );
}

#[test]
fn situation_filter_and_both_together() {
    let c = Scenario::new("situacao");
    let org = c.org("a");
    let agent = c.agent(&org, "a");
    let opened = c.conversation(&org, &agent, Some("5511"));
    let transferred = c.conversation(&org, &agent, Some("5511"));
    let closed = c.conversation(&org, &agent, Some("5522"));
    let transferred_from_other = c.conversation(&org, &agent, Some("5522"));

    c.store
        .conversations()
        .transfer(org.id, transferred.id, None, None, "{}".to_owned())
        .expect("transfere");
    c.store
        .conversations()
        .change_status(org.id, closed.id, 1, ConversationStatus::Closed, None)
        .expect("fecha");
    c.store
        .conversations()
        .transfer(
            org.id,
            transferred_from_other.id,
            None,
            None,
            "{}".to_owned(),
        )
        .expect("transfere a quarta");

    let only_transferred = FilterConversations {
        status: Some(ConversationStatus::Transferred),
        external_peer: None,
        ..Default::default()
    };
    assert_eq!(
        c.list(org.id, &only_transferred),
        vec![transferred.id, transferred_from_other.id]
    );

    let only_closed = FilterConversations {
        status: Some(ConversationStatus::Closed),
        external_peer: None,
        ..Default::default()
    };
    assert_eq!(c.list(org.id, &only_closed), vec![closed.id]);

    let only_open = FilterConversations {
        status: Some(ConversationStatus::Open),
        external_peer: None,
        ..Default::default()
    };
    assert_eq!(c.list(org.id, &only_open), vec![opened.id]);

    let client_and_transferred = FilterConversations {
        status: Some(ConversationStatus::Transferred),
        external_peer: Some("5511".to_owned()),
        ..Default::default()
    };
    assert_eq!(
        c.list(org.id, &client_and_transferred),
        vec![transferred.id],
        "cliente e situação juntos devolvem só a conversa que atende os dois"
    );
}

#[test]
fn without_filter_listing_is_by_id_with_cursor() {
    let c = Scenario::new("sem-filtro");
    let org = c.org("a");
    let agent = c.agent(&org, "a");
    let ids: Vec<ConversationId> = (0..5)
        .map(|n| {
            c.conversation(&org, &agent, Some(format!("55{n}").as_str()))
                .id
        })
        .collect();

    let empty = FilterConversations::default();
    let first = c
        .store
        .conversations()
        .list_filter(org.id, &empty, Page::with_limit(2))
        .expect("página 1");
    assert_eq!(first.iter().map(|c| c.id).collect::<Vec<_>>(), ids[..2]);

    let second = c
        .store
        .conversations()
        .list_filter(
            org.id,
            &empty,
            Page {
                after: Some(ids[1].as_uuid()),
                limit: 2,
            },
        )
        .expect("página 2");
    assert_eq!(
        second.iter().map(|c| c.id).collect::<Vec<_>>(),
        ids[2..4],
        "o cursor é o último id da página anterior"
    );
}

#[test]
fn recent_order_uses_activity_breaks_ties_by_id_and_respects_limit() {
    let c = Scenario::new("recente");
    let org = c.org("a");
    let agent = c.agent(&org, "a");
    let c1 = c.conversation(&org, &agent, Some("5511"));
    let c2 = c.conversation(&org, &agent, Some("5522"));
    let c3 = c.conversation(&org, &agent, Some("5533"));

    c.store
        .conversations()
        .touch(org.id, c1.id, 9_000_000)
        .expect("toca c1");
    c.store
        .conversations()
        .touch(org.id, c3.id, 9_000_000)
        .expect("toca c3");

    let without = FilterConversations::default();
    let sorted: Vec<ConversationId> = c
        .store
        .conversations()
        .list_recents(org.id, &without, 50)
        .expect("recentes")
        .iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(
        sorted,
        vec![c3.id, c1.id, c2.id],
        "atividade decrescente; empate na mesma atividade vai pelo id maior primeiro"
    );

    let two = c
        .store
        .conversations()
        .list_recents(org.id, &without, 2)
        .expect("limite 2");
    assert_eq!(
        two.iter().map(|c| c.id).collect::<Vec<_>>(),
        vec![c3.id, c1.id]
    );

    let only_client = c
        .store
        .conversations()
        .list_recents(org.id, &filter_peer("5522"), 50)
        .expect("recentes do cliente");
    assert_eq!(
        only_client.iter().map(|c| c.id).collect::<Vec<_>>(),
        vec![c2.id],
        "a ordem recente também respeita o filtro por cliente"
    );
}

#[test]
fn customer_query_plan_uses_new_index_and_does_not_scan_table() {
    let c = Scenario::new("plano");
    let org = c.org("a");
    let agent = c.agent(&org, "a");
    for n in 0..3 {
        c.conversation(&org, &agent, Some(format!("55{n}").as_str()));
    }

    let by_client = FilterConversations {
        external_peer: Some("5511".to_owned()),
        status: Some(ConversationStatus::Open),
        ..Default::default()
    };
    for ordem in [OrderConversations::Id, OrderConversations::Recent] {
        let plan = c
            .store
            .conversations()
            .listing_plan(org.id, &by_client, ordem)
            .expect("plano");
        let texto = plan.join(" | ");
        assert!(
            texto.contains("ix_conversations_org_peer_id"),
            "o plano não usa o índice novo ({ordem:?}): {texto}"
        );
        assert!(
            texto.contains("external_peer=?"),
            "a busca não usa a coluna do cliente ({ordem:?}): {texto}"
        );
        if ordem == OrderConversations::Id {
            assert!(
                !texto.contains("TEMP B-TREE"),
                "a ordem por id sai do índice, sem ordenação temporária: {texto}"
            );
        }
        assert!(
            !texto.contains("SCAN conversations"),
            "a consulta varre a tabela inteira ({ordem:?}): {texto}"
        );
    }
}

#[test]
fn customer_identifier_is_peer_without_space_or_id_when_empty() {
    let c = Scenario::new("identificador");
    let org = c.org("a");
    let agent = c.agent(&org, "a");

    let normal = c.conversation(&org, &agent, Some("5511999999999"));
    assert_eq!(normal.identifier_client(), "5511999999999");

    let with_spaces = c.conversation(&org, &agent, Some("  5511999999999  "));
    assert_eq!(
        with_spaces.identifier_client(),
        "5511999999999",
        "o identificador sai sem espaço nas pontas"
    );

    let empty = c.conversation(&org, &agent, Some("   "));
    assert_eq!(
        empty.identifier_client(),
        empty.id.to_string(),
        "peer só com espaço cai no id da conversa"
    );

    let without_peer = c.conversation(&org, &agent, None);
    assert_eq!(
        without_peer.identifier_client(),
        without_peer.id.to_string()
    );

    let found_items = c.list(org.id, &filter_peer(&normal.identifier_client()));
    assert!(
        found_items.contains(&normal.id),
        "o identificador da sala acha a conversa pelo filtro de cliente"
    );
}

#[test]
fn last_activity_filter_brings_only_stalled_conversations_of_org() {
    let c = Scenario::new("inatividade");
    let org = c.org("a");
    let other = c.org("b");
    let agent = c.agent(&org, "a");
    let agent_b = c.agent(&other, "b");
    let first = c.conversation(&org, &agent, Some("5511"));
    let second = c.conversation(&org, &agent, Some("5522"));
    let third = c.conversation(&org, &agent, Some("5533"));
    let foreign = c.conversation(&other, &agent_b, Some("5511"));
    assert!(foreign.created_at >= third.created_at);

    let stalled_until = FilterConversations {
        last_activity_before: Some(second.created_at + 1),
        ..Default::default()
    };
    assert_eq!(c.list(org.id, &stalled_until), vec![first.id, second.id]);

    let nothing = FilterConversations {
        last_activity_before: Some(first.created_at),
        ..Default::default()
    };
    assert!(c.list(org.id, &nothing).is_empty());

    let open_stalled = FilterConversations {
        status: Some(ConversationStatus::Open),
        external_peer: None,
        last_activity_before: Some(second.created_at + 1),
    };
    assert_eq!(c.list(org.id, &open_stalled), vec![first.id, second.id]);

    c.store
        .conversations()
        .change_status(org.id, first.id, 1, ConversationStatus::Closed, None)
        .expect("fecha a primeira");
    assert_eq!(c.list(org.id, &open_stalled), vec![second.id]);
}
