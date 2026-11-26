use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_store::{
    Agente, CAP_FLOWS_BY_AGENT, Conversa, FlowChange, FlowFilter, Fluxo, NewAgent, NewConversation,
    NewFlow, NewOrg, Org, Page, REASON_EXCLUSION_FLOW, Status, Store, StoreError,
};
use poph_types::{AgentId, DomainError, EventActor, FlowId, OrgId, RecordEventKind, RecordId};
use rusqlite::Connection;

struct Scenario {
    folder: PathBuf,
    store: Option<Store>,
}

impl Scenario {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let folder =
            std::env::temp_dir().join(format!("poph-fluxos-{label}-{}-{n}", std::process::id()));
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

    fn connection(&self) -> Connection {
        let conn = Connection::open(self.folder.join("harness.db")).expect("abre para conferir");
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
            .expect("pragmas");
        conn
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

    fn agent(&self, org: &Org, slug: &str) -> Agente {
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

    fn flow(&self, org: &Org, slug: &str) -> Fluxo {
        self.store()
            .flows()
            .create(
                org.id,
                NewFlow {
                    slug: slug.to_owned(),
                    name: format!("Fluxo {slug}"),
                    description: String::new(),
                    rascunho_fonte: String::new(),
                },
            )
            .expect("cria o fluxo")
    }

    fn agent_version(&self, org: &Org, agent: &Agente) -> u64 {
        self.store()
            .agents()
            .get(org.id, agent.id)
            .expect("lê o agente")
            .version
    }

    fn conversation(&self, org: &Org, agent: &Agente) -> Conversa {
        self.store()
            .conversations()
            .create(
                org.id,
                NewConversation {
                    agent_id: agent.id,
                    integration_id: None,
                    canal: "console".to_owned(),
                    peer_externo: Some("cliente-1".to_owned()),
                    target_department: None,
                },
            )
            .expect("abre a conversa")
    }

    fn execution_active(&self, org: OrgId, case: RecordId, flow: FlowId) -> String {
        let execution = uuid::Uuid::now_v7().to_string();
        let revision = uuid::Uuid::now_v7().to_string();
        self.connection()
            .execute(
                "INSERT INTO fluxo_execucoes
                    (id, org_id, ficha_id, fluxo_id, revisao_id, status, no_atual, iniciada_em)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'ativa', 'inicio', 1)",
                rusqlite::params![
                    execution,
                    org.to_string(),
                    case.to_string(),
                    flow.to_string(),
                    revision
                ],
            )
            .expect("execução ativa");
        execution
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = self.store.take();
        let _ = fs::remove_dir_all(&self.folder);
    }
}

fn domain<T: std::fmt::Debug>(resultado: Result<T, StoreError>) -> DomainError {
    match resultado {
        Err(StoreError::Domain(error)) => error,
        another => panic!("esperado erro de domínio, veio {another:?}"),
    }
}

#[test]
fn flow_crud_with_listing_filter_and_cursor() {
    let scenario = Scenario::new("crud");
    let org = scenario.org("a");
    let store = scenario.store();

    let first_value = scenario.flow(&org, "agendar_visita");
    assert_eq!(first_value.status, Status::Active);
    assert_eq!(first_value.revisao_atual, None);
    assert_eq!(first_value.version, 1);

    let obtained = store.flows().get(org.id, first_value.id).expect("obtém");
    assert_eq!(obtained, first_value);
    let by_slug = store
        .flows()
        .by_slug(org.id, "agendar_visita")
        .expect("por slug");
    assert_eq!(by_slug, Some(first_value.clone()));
    assert_eq!(
        store
            .flows()
            .by_slug(org.id, "nao_existe")
            .expect("por slug"),
        None
    );

    let second_value = scenario.flow(&org, "triagem_suporte");
    scenario.flow(&org, "confirmar_entrega");
    store
        .flows()
        .update(
            org.id,
            second_value.id,
            1,
            FlowChange {
                status: Some(Status::Inactive),
                ..FlowChange::default()
            },
        )
        .expect("desativa");

    let active_rows = store
        .flows()
        .list(
            org.id,
            &FlowFilter {
                status: Some(Status::Active),
            },
            Page::default(),
        )
        .expect("lista ativos");
    assert_eq!(active_rows.len(), 2);
    assert!(active_rows.iter().all(|flow| flow.status == Status::Active));

    let all_rows = store
        .flows()
        .list(org.id, &FlowFilter::default(), Page::default())
        .expect("lista todos");
    assert_eq!(all_rows.len(), 3);

    let page_1 = store
        .flows()
        .list(org.id, &FlowFilter::default(), Page::with_limit(2))
        .expect("página 1");
    assert_eq!(page_1.len(), 2);
    let after: uuid::Uuid = page_1[1].id.to_string().parse().expect("uuid");
    let page_2 = store
        .flows()
        .list(
            org.id,
            &FlowFilter::default(),
            Page {
                after: Some(after),
                limit: 2,
            },
        )
        .expect("página 2");
    assert_eq!(page_2.len(), 1);
    assert_eq!(page_2[0].id, all_rows[2].id);
}

#[test]
fn update_uses_if_match_and_refuses_old_version() {
    let scenario = Scenario::new("if-match");
    let org = scenario.org("a");
    let flow = scenario.flow(&org, "agendar_visita");
    let store = scenario.store();

    let updated = store
        .flows()
        .update(
            org.id,
            flow.id,
            1,
            FlowChange {
                name: Some("Agendar visita técnica".to_owned()),
                rascunho_fonte: Some("fluxo(id = \"agendar_visita\")".to_owned()),
                ..FlowChange::default()
            },
        )
        .expect("com a versão certa atualiza");
    assert_eq!(updated.version, 2);
    assert_eq!(updated.name, "Agendar visita técnica");
    assert_eq!(updated.rascunho_fonte, "fluxo(id = \"agendar_visita\")");

    let error = domain(store.flows().update(
        org.id,
        flow.id,
        1,
        FlowChange {
            name: Some("Outro nome".to_owned()),
            ..FlowChange::default()
        },
    ));
    assert!(
        matches!(
            error,
            DomainError::VersionConflict {
                expected: 1,
                current: 2,
                ..
            }
        ),
        "erro inesperado: {error:?}"
    );
    let later = store.flows().get(org.id, flow.id).expect("obtém");
    assert_eq!(
        later.name, "Agendar visita técnica",
        "a escrita recusada não gravou"
    );
    assert_eq!(later.version, 2);
}

#[test]
fn archived_situation_is_refused_in_flow() {
    let scenario = Scenario::new("arquivado");
    let org = scenario.org("a");
    let flow = scenario.flow(&org, "agendar_visita");

    let error = domain(scenario.store().flows().update(
        org.id,
        flow.id,
        1,
        FlowChange {
            status: Some(Status::Archived),
            ..FlowChange::default()
        },
    ));
    assert!(
        matches!(
            &error,
            DomainError::Validation {
                field: "status",
                ..
            }
        ),
        "erro inesperado: {error:?}"
    );
}

#[test]
fn repeated_slug_is_refused_in_org_and_accepted_in_another() {
    let scenario = Scenario::new("slug");
    let org_a = scenario.org("a");
    let org_b = scenario.org("b");
    scenario.flow(&org_a, "agendar_visita");

    let error = domain(scenario.store().flows().create(
        org_a.id,
        NewFlow {
            slug: "agendar_visita".to_owned(),
            name: "Repetido".to_owned(),
            ..NewFlow::default()
        },
    ));
    assert!(
        matches!(&error, DomainError::Validation { field: "slug", .. }),
        "erro inesperado: {error:?}"
    );

    let another = scenario.flow(&org_b, "agendar_visita");
    assert_eq!(another.org_id, org_b.id);
}

#[test]
fn slug_outside_language_format_is_refused() {
    let scenario = Scenario::new("formato");
    let org = scenario.org("a");
    for slug in [
        "",
        "a",
        "Agendar",
        "1visita",
        "agendar-visita",
        "agendar visita",
    ] {
        let error = domain(scenario.store().flows().create(
            org.id,
            NewFlow {
                slug: slug.to_owned(),
                name: "Nome".to_owned(),
                ..NewFlow::default()
            },
        ));
        assert!(
            matches!(&error, DomainError::Validation { field: "slug", .. }),
            "slug `{slug}` devia ser recusado, veio {error:?}"
        );
    }
}

#[test]
fn deleting_without_active_execution_returns_zero_and_takes_permission() {
    let scenario = Scenario::new("excluir-simples");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let flow = scenario.flow(&org, "agendar_visita");
    let store = scenario.store();
    store
        .flows()
        .release(org.id, agent.id, flow.id)
        .expect("libera");

    let active_list = store
        .flows()
        .delete(org.id, flow.id, false)
        .expect("exclui sem execução ativa");
    assert_eq!(active_list, 0);
    assert!(matches!(
        domain(store.flows().get(org.id, flow.id)),
        DomainError::NotFound {
            entity: "fluxo",
            ..
        }
    ));
    assert!(
        store
            .flows()
            .list_from_agent(org.id, agent.id)
            .expect("lista")
            .is_empty(),
        "a permissão do fluxo excluído tem que sair junto"
    );
    assert!(
        store.agents().get(org.id, agent.id).is_ok(),
        "excluir fluxo não apaga agente"
    );
}

#[test]
fn deleting_with_active_execution_requires_force_and_abandons_keeping_history() {
    let scenario = Scenario::new("excluir-ativa");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let flow = scenario.flow(&org, "agendar_visita");
    let store = scenario.store();
    store
        .flows()
        .release(org.id, agent.id, flow.id)
        .expect("libera");
    let conversation = scenario.conversation(&org, &agent);
    let case = store
        .cases()
        .by_conversation(org.id, conversation.id)
        .expect("consulta")
        .expect("a conversa tem Ficha")
        .id;
    let execution = scenario.execution_active(org.id, case, flow.id);

    let error = domain(store.flows().delete(org.id, flow.id, false));
    assert!(
        matches!(&error, DomainError::Conflict { entity: "fluxo", reason, .. } if reason.contains("1 execução")),
        "erro inesperado: {error:?}"
    );
    store
        .flows()
        .get(org.id, flow.id)
        .expect("o fluxo continua existindo depois da recusa");
    let situation_before: String = scenario
        .connection()
        .query_row(
            "SELECT status FROM fluxo_execucoes WHERE id = ?1",
            [&execution],
            |row| row.get(0),
        )
        .expect("lê a execução");
    assert_eq!(situation_before, "ativa");

    let active_list = store
        .flows()
        .delete(org.id, flow.id, true)
        .expect("exclui com force");
    assert_eq!(active_list, 1, "devolve quantas execuções ativas havia");
    assert!(matches!(
        domain(store.flows().get(org.id, flow.id)),
        DomainError::NotFound { .. }
    ));

    let conn = scenario.connection();
    let (status, motivo, finalised): (String, Option<String>, Option<i64>) = conn
        .query_row(
            "SELECT status, motivo_fim, finalizada_em FROM fluxo_execucoes WHERE id = ?1",
            [&execution],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("a execução continua no banco: é histórico do atendimento");
    assert_eq!(status, "abandonada");
    assert_eq!(motivo.as_deref(), Some(REASON_EXCLUSION_FLOW));
    assert!(
        finalised.is_some(),
        "execução encerrada tem instante de fim"
    );

    store.cases().get(org.id, case).expect("a Ficha permanece");
    let events = store
        .cases()
        .events(org.id, case, None, 100)
        .expect("eventos");
    let abandono = events
        .iter()
        .find(|evento| evento.tipo == RecordEventKind::ExecutionAbandoned)
        .expect("o abandono entra na linha do tempo da Ficha");
    assert_eq!(abandono.ator_tipo, EventActor::System);
    assert_eq!(
        abandono.execucao_id.map(|id| id.to_string()),
        Some(execution.clone())
    );
    assert!(abandono.payload_json.contains(REASON_EXCLUSION_FLOW));

    assert!(
        store
            .flows()
            .list_from_agent(org.id, agent.id)
            .expect("lista")
            .is_empty()
    );
}

#[test]
fn deleting_missing_flow_or_from_other_org_is_not_found() {
    let scenario = Scenario::new("excluir-ausente");
    let org_a = scenario.org("a");
    let org_b = scenario.org("b");
    let flow_b = scenario.flow(&org_b, "agendar_visita");

    let error = domain(scenario.store().flows().delete(org_a.id, flow_b.id, true));
    assert!(matches!(
        error,
        DomainError::NotFound {
            entity: "fluxo",
            ..
        }
    ));
    scenario
        .store()
        .flows()
        .get(org_b.id, flow_b.id)
        .expect("intacto");
}

#[test]
fn flow_cap_per_agent_refuses_21st_and_repeating_does_not_count() {
    let scenario = Scenario::new("teto");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let store = scenario.store();
    let flows: Vec<Fluxo> = (0..21)
        .map(|n| scenario.flow(&org, &format!("fluxo_{n:02}")))
        .collect();

    for flow in &flows[..20] {
        store
            .flows()
            .release(org.id, agent.id, flow.id)
            .expect("até o 20º libera");
    }
    store
        .flows()
        .release(org.id, agent.id, flows[0].id)
        .expect("repetir não conta de novo");
    assert_eq!(
        store
            .flows()
            .list_from_agent(org.id, agent.id)
            .expect("lista")
            .len(),
        20
    );

    let error = domain(store.flows().release(org.id, agent.id, flows[20].id));
    assert_eq!(
        error,
        DomainError::FlowLimitPerAgent {
            agent_id: agent.id.to_string(),
            limit: CAP_FLOWS_BY_AGENT,
            requested: 21,
        }
    );
    assert_eq!(error.api_code(), "teto_de_fluxos_por_agente");
    assert_eq!(error.http_status(), 409);
    assert_eq!(
        store
            .flows()
            .list_from_agent(org.id, agent.id)
            .expect("lista")
            .len(),
        20,
        "o 21º não entrou"
    );
}

#[test]
fn replacing_list_with_21_is_refused_and_keeps_old_list() {
    let scenario = Scenario::new("teto-lote");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let store = scenario.store();
    let old = scenario.flow(&org, "fluxo_antigo");
    store
        .flows()
        .release(org.id, agent.id, old.id)
        .expect("libera o antigo");
    let new_rows: Vec<FlowId> = (0..21)
        .map(|n| scenario.flow(&org, &format!("fluxo_{n:02}")).id)
        .collect();
    let version = scenario.agent_version(&org, &agent);

    let error = domain(
        store
            .flows()
            .replace_list(org.id, agent.id, version, &new_rows),
    );
    assert!(
        matches!(
            error,
            DomainError::FlowLimitPerAgent {
                limit: 20,
                requested: 21,
                ..
            }
        ),
        "erro inesperado: {error:?}"
    );
    let list = store
        .flows()
        .list_from_agent(org.id, agent.id)
        .expect("lista");
    assert_eq!(list.len(), 1, "a lista antiga continua inteira");
    assert_eq!(list[0].id, old.id);
    assert_eq!(
        scenario.agent_version(&org, &agent),
        version,
        "versão não avançou"
    );
}

#[test]
fn replacing_list_is_atomic_and_respects_agent_if_match() {
    let scenario = Scenario::new("substituir");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let store = scenario.store();
    let a = scenario.flow(&org, "fluxo_a");
    let b = scenario.flow(&org, "fluxo_b");
    let c = scenario.flow(&org, "fluxo_c");

    let v1 = scenario.agent_version(&org, &agent);
    store
        .flows()
        .replace_list(org.id, agent.id, v1, &[a.id, b.id, a.id])
        .expect("troca a lista");
    let v2 = scenario.agent_version(&org, &agent);
    assert_eq!(v2, v1 + 1, "troca de permissão avança a versão do agente");
    let list: Vec<FlowId> = store
        .flows()
        .list_from_agent(org.id, agent.id)
        .expect("lista")
        .into_iter()
        .map(|flow| flow.id)
        .collect();
    assert_eq!(list.len(), 2);
    assert!(list.contains(&a.id) && list.contains(&b.id));

    store
        .flows()
        .replace_list(org.id, agent.id, v2, &[a.id, b.id])
        .expect("mesma lista de novo");
    let v3 = scenario.agent_version(&org, &agent);

    let error = domain(store.flows().replace_list(org.id, agent.id, v1, &[c.id]));
    assert!(
        matches!(error, DomainError::VersionConflict { .. }),
        "erro inesperado: {error:?}"
    );
    assert_eq!(scenario.agent_version(&org, &agent), v3);
    assert_eq!(
        store
            .flows()
            .list_from_agent(org.id, agent.id)
            .expect("lista")
            .len(),
        2
    );
}

#[test]
fn release_and_revoke_advance_agent_version_and_revoke_is_idempotent() {
    let scenario = Scenario::new("versao-agente");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let flow = scenario.flow(&org, "agendar_visita");
    let store = scenario.store();
    let v0 = scenario.agent_version(&org, &agent);

    store
        .flows()
        .release(org.id, agent.id, flow.id)
        .expect("libera");
    let v1 = scenario.agent_version(&org, &agent);
    assert_eq!(v1, v0 + 1);

    store
        .flows()
        .revoke(org.id, agent.id, flow.id)
        .expect("revoga");
    let v2 = scenario.agent_version(&org, &agent);
    assert_eq!(v2, v1 + 1);

    store
        .flows()
        .revoke(org.id, agent.id, flow.id)
        .expect("revogar de novo não falha");
    assert_eq!(scenario.agent_version(&org, &agent), v2);
}

#[test]
fn listing_agents_of_flow_returns_who_has_permission() {
    let scenario = Scenario::new("agentes-do-fluxo");
    let org = scenario.org("a");
    let agent_1 = scenario.agent(&org, "um");
    let agent_2 = scenario.agent(&org, "dois");
    let flow = scenario.flow(&org, "agendar_visita");
    let store = scenario.store();
    store
        .flows()
        .release(org.id, agent_1.id, flow.id)
        .expect("libera 1");
    store
        .flows()
        .release(org.id, agent_2.id, flow.id)
        .expect("libera 2");

    let mut who: Vec<AgentId> = store
        .flows()
        .list_agents_of_flow(org.id, flow.id)
        .expect("lista");
    who.sort();
    let mut expected_values = vec![agent_1.id, agent_2.id];
    expected_values.sort();
    assert_eq!(who, expected_values);
}

#[test]
fn permission_with_flow_or_agent_of_other_org_is_refused() {
    let scenario = Scenario::new("org-errada");
    let org_a = scenario.org("a");
    let org_b = scenario.org("b");
    let agent_a = scenario.agent(&org_a, "a");
    let agent_b = scenario.agent(&org_b, "b");
    let flow_a = scenario.flow(&org_a, "fluxo_a");
    let flow_b = scenario.flow(&org_b, "fluxo_b");
    let store = scenario.store();

    let error = domain(store.flows().release(org_a.id, agent_a.id, flow_b.id));
    assert!(
        matches!(
            &error,
            DomainError::NotFound {
                entity: "fluxo",
                ..
            }
        ),
        "erro inesperado: {error:?}"
    );
    let error = domain(store.flows().release(org_a.id, agent_b.id, flow_a.id));
    assert!(
        matches!(
            &error,
            DomainError::NotFound {
                entity: "agente",
                ..
            }
        ),
        "erro inesperado: {error:?}"
    );
    let error = domain(
        store
            .flows()
            .release(org_a.id, agent_a.id, FlowId::generate()),
    );
    assert!(matches!(
        error,
        DomainError::NotFound {
            entity: "fluxo",
            ..
        }
    ));

    store
        .flows()
        .release(org_a.id, agent_a.id, flow_a.id)
        .expect("libera o próprio");
    let version = scenario.agent_version(&org_a, &agent_a);
    let error = domain(
        store
            .flows()
            .replace_list(org_a.id, agent_a.id, version, &[flow_b.id]),
    );
    assert!(matches!(
        &error,
        DomainError::NotFound {
            entity: "fluxo",
            ..
        }
    ));
    let list = store
        .flows()
        .list_from_agent(org_a.id, agent_a.id)
        .expect("lista");
    assert_eq!(list.len(), 1, "a lista antiga não foi apagada");
    assert_eq!(list[0].id, flow_a.id);
    assert_eq!(
        scenario.agent_version(&org_a, &agent_a),
        version,
        "a versão do agente também não avançou"
    );

    let error = domain(store.flows().list_agents_of_flow(org_a.id, flow_b.id));
    assert!(matches!(error, DomainError::NotFound { .. }));
}

#[test]
fn deleting_agent_removes_only_permission() {
    let scenario = Scenario::new("apaga-agente");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let flow = scenario.flow(&org, "agendar_visita");
    let store = scenario.store();
    store
        .flows()
        .release(org.id, agent.id, flow.id)
        .expect("libera");

    scenario
        .connection()
        .execute(
            "DELETE FROM agents WHERE org_id = ?1 AND id = ?2",
            rusqlite::params![org.id.to_string(), agent.id.to_string()],
        )
        .expect("apaga o agente");

    let ainda = store
        .flows()
        .get(org.id, flow.id)
        .expect("o fluxo continua existindo");
    assert_eq!(ainda.id, flow.id);
    let permissions: i64 = scenario
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM agente_fluxos WHERE org_id = ?1 AND agent_id = ?2",
            rusqlite::params![org.id.to_string(), agent.id.to_string()],
            |row| row.get(0),
        )
        .expect("conta");
    assert_eq!(permissions, 0, "apagar o agente leva a permissão");
}

#[test]
fn deleting_flow_takes_revisions_with_it() {
    let scenario = Scenario::new("revisao-cascata");
    let org = scenario.org("a");
    let flow = scenario.flow(&org, "agendar_visita");
    let conn = scenario.connection();
    conn.execute(
        "INSERT INTO fluxo_revisoes
            (id, org_id, fluxo_id, numero, fonte, fonte_sha256, definicao_json, publicada_em,
             publicada_por)
         VALUES (?1, ?2, ?3, 1, 'fluxo(id = \"agendar_visita\")', 'sha', '{}', 1, 'cli')",
        rusqlite::params![
            uuid::Uuid::now_v7().to_string(),
            org.id.to_string(),
            flow.id.to_string()
        ],
    )
    .expect("revisão");

    scenario
        .store()
        .flows()
        .delete(org.id, flow.id, false)
        .expect("exclui: a revisão sai junto com o fluxo");
    let restantes: i64 = conn
        .query_row("SELECT COUNT(*) FROM fluxo_revisoes", [], |row| row.get(0))
        .expect("conta");
    assert_eq!(restantes, 0);
}
