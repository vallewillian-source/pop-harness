use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

use poph_store::{
    AgentChange, Agente, CategoryChange, Conversa, ConversationStatus, DepartmentChange, Direction,
    ExecutionStart, ExecutionState, FlowChange, FlowFilter, FlowTransition, NewAgent,
    NewAuditEntry, NewCategory, NewConversation, NewDepartment, NewFlow, NewInboxItem, NewMessage,
    NewOrg, NewOutboxItem, NewRecord, NewRecordEvent, Org, OrgChange, OutboxState, Page, Received,
    RevisionPublication, Role, Status, Store, StoreError, TransitionEffect, TransitionExtras,
    file_version,
};
use poph_types::{
    DomainError, EventActor, ExecutionStatus, InboxId, IntegrationId, MessageId, OrgId,
    RecordEventKind,
};

struct Scenario {
    folder: PathBuf,
    store: Option<Arc<Store>>,
}

impl Scenario {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let folder =
            std::env::temp_dir().join(format!("poph-store-{label}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("cria a pasta do teste");
        let store = Store::open_at(&folder.join("harness.db"), &folder.join("rag.db"))
            .expect("abre o banco");
        Self {
            folder,
            store: Some(Arc::new(store)),
        }
    }

    fn store(&self) -> &Store {
        self.store.as_deref().expect("store viva")
    }

    fn shared(&self) -> Arc<Store> {
        Arc::clone(self.store.as_ref().expect("store viva"))
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = self.store.take();
        let _ = fs::remove_dir_all(&self.folder);
    }
}

fn create_org(store: &Store, slug: &str) -> Org {
    store
        .orgs()
        .create(NewOrg {
            name: format!("Org {slug}"),
            slug: slug.to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org")
}

fn create_agent(store: &Store, org: &Org, slug: &str) -> Agente {
    let new = NewAgent {
        name: format!("Agente {slug}"),
        slug: slug.to_owned(),
        ..NewAgent::default()
    };
    store.agents().create(org.id, new).expect("cria o agente")
}

fn create_conversation(store: &Store, org: &Org, agent: &Agente) -> Conversa {
    store
        .conversations()
        .create(
            org.id,
            NewConversation {
                agent_id: agent.id,
                integration_id: None,
                canal: "cli".to_owned(),
                peer_externo: Some("teste".to_owned()),
                target_department: None,
            },
        )
        .expect("cria a conversa")
}

fn write_message(store: &Store, org: &Org, conversation: &Conversa, texto: &str) -> MessageId {
    store
        .messages()
        .write(
            org.id,
            NewMessage {
                conversation_id: conversation.id,
                direction: Direction::Inbound,
                role: Role::User,
                texto: texto.to_owned(),
                attachments_json: "[]".to_owned(),
                external_id: None,
                turn_id: None,
                origin: poph_store::Origin::Agente,
                author_name: None,
            },
        )
        .expect("grava a mensagem")
        .id
}

#[test]
fn migration_creates_prd_schema_and_is_idempotent() {
    let scenario = Scenario::new("migracao");
    let store = scenario.store();

    let tabelas = store.tables().expect("lista tabelas");
    for expected in [
        "orgs",
        "admin_tokens",
        "product_categories",
        "products",
        "departments",
        "agents",
        "agent_product_categories",
        "agent_instructions",
        "knowledge_groups",
        "knowledge_items",
        "agent_knowledge_groups",
        "file_library",
        "integrations",
        "conversations",
        "messages",
        "attachments",
        "carts",
        "cart_items",
        "inbox",
        "outbox",
        "turns",
        "audit_log",
        "matrix_orgs",
        "matrix_rooms",
        "matrix_spaces",
        "matrix_estado",
        "matrix_falhas",
        "schema_migrations",
    ] {
        assert!(
            tabelas.iter().any(|table| table == expected),
            "tabela {expected} não criada; existem: {tabelas:?}"
        );
    }

    let path = store.harness_path().to_path_buf();
    assert_eq!(
        file_version(&path).expect("lê versão"),
        Some(poph_store::harness_target_version())
    );
    assert!(store.broken_foreign_keys().expect("fk check").is_empty());

    let reports = store.migrate().expect("migra de novo");
    assert!(reports.already_up_to_date(), "{reports:?}");
    assert_eq!(
        file_version(&path).expect("lê versão"),
        Some(poph_store::harness_target_version())
    );

    let rag_path = store.rag_path().to_path_buf();
    assert_eq!(file_version(&rag_path).expect("lê versão"), Some(0));
}

#[test]
fn every_business_table_has_org_id() {
    let scenario = Scenario::new("org-id");
    let store = scenario.store();
    let without_org = ["orgs", "schema_migrations"];
    let mut verified = 0;
    for table in store.tables().expect("lista tabelas") {
        if without_org.contains(&table.as_str()) {
            continue;
        }
        let columns = store.columns(&table).expect("lê colunas");
        assert!(
            columns.iter().any(|column| column == "org_id"),
            "tabela {table} não tem org_id; colunas: {columns:?}"
        );
        verified += 1;
    }
    assert!(verified >= 20, "poucas tabelas verificadas: {verified}");
}

#[test]
fn no_repository_reads_record_of_other_org() {
    let scenario = Scenario::new("isolamento");
    let store = scenario.store();
    let org_a = create_org(store, "org-a");
    let org_b = create_org(store, "org-b");

    let category = store
        .categories()
        .create(
            org_a.id,
            NewCategory {
                name: "Dedetização".to_owned(),
                description: String::new(),
            },
        )
        .expect("categoria");
    let departamento = store
        .departments()
        .create(
            org_a.id,
            NewDepartment {
                name: "Comercial".to_owned(),
                description: String::new(),
            },
        )
        .expect("departamento");
    let agent = create_agent(store, &org_a, "agente-a");
    store
        .agents()
        .define_categories(org_a.id, agent.id, agent.version, &[category.id])
        .expect("vincula categoria");
    store
        .instructions()
        .save(org_a.id, agent.id, 0, "{}", "# A", true)
        .expect("instruções");
    let conversation = create_conversation(store, &org_a, &agent);
    let mensagem = write_message(store, &org_a, &conversation, "oi");
    let turno = store
        .turns()
        .start(org_a.id, conversation.id, Some(agent.id))
        .expect("turno");
    let entry = store
        .inbox()
        .receive(
            org_a.id,
            NewInboxItem {
                integration_id: IntegrationId::generate(),
                external_id: "wamid-1".to_owned(),
                event_json: "{}".to_owned(),
            },
        )
        .expect("entrada")
        .1;
    let saida = store
        .outbox()
        .enqueue(
            org_a.id,
            NewOutboxItem {
                conversation_id: conversation.id,
                turn_id: turno.id,
                seq: 0,
                tipo: "text".to_owned(),
                content_json: "{}".to_owned(),
            },
        )
        .expect("saída")
        .1;
    let token = store
        .tokens()
        .create(Some(org_a.id), "token A", "hash-a")
        .expect("token");
    store
        .audit()
        .register(
            Some(org_a.id),
            NewAuditEntry {
                ator_tipo: "system".to_owned(),
                actor_id: None,
                acao: "create".to_owned(),
                entity: "agent".to_owned(),
                entity_id: Some(agent.id.to_string()),
                detail_json: "{}".to_owned(),
            },
        )
        .expect("auditoria");

    let not_found = |resultado: Result<(), poph_store::StoreError>| match resultado {
        Err(poph_store::StoreError::Domain(DomainError::NotFound { .. })) => true,
        another => panic!("esperado não encontrado, veio {another:?}"),
    };

    not_found(store.categories().get(org_b.id, category.id).map(|_| ()));
    assert!(
        store
            .categories()
            .list(org_b.id, Page::default())
            .expect("lista")
            .is_empty()
    );
    not_found(
        store
            .departments()
            .get(org_b.id, departamento.id)
            .map(|_| ()),
    );
    assert!(
        store
            .departments()
            .list(org_b.id, Page::default())
            .expect("lista")
            .is_empty()
    );
    not_found(store.agents().get(org_b.id, agent.id).map(|_| ()));
    assert!(
        store
            .agents()
            .list(org_b.id, Page::default())
            .expect("lista")
            .is_empty()
    );
    assert!(
        store
            .agents()
            .categories(org_b.id, agent.id)
            .expect("categorias")
            .is_empty()
    );
    not_found(store.instructions().get(org_b.id, agent.id).map(|_| ()));
    not_found(
        store
            .conversations()
            .get(org_b.id, conversation.id)
            .map(|_| ()),
    );
    assert!(
        store
            .conversations()
            .list(org_b.id, None, Page::default())
            .expect("lista")
            .is_empty()
    );
    not_found(
        store
            .conversations()
            .agent(org_b.id, conversation.id)
            .map(|_| ()),
    );
    assert!(
        store
            .messages()
            .list(org_b.id, conversation.id, Page::default())
            .expect("lista")
            .is_empty()
    );
    not_found(
        store
            .messages()
            .get(org_b.id, conversation.id, mensagem)
            .map(|_| ()),
    );
    assert_eq!(
        store
            .messages()
            .count_rows(org_b.id, conversation.id)
            .expect("conta"),
        0
    );
    not_found(store.turns().get(org_b.id, turno.id).map(|_| ()));
    assert!(
        store
            .turns()
            .list(org_b.id, conversation.id, Page::default())
            .expect("lista")
            .is_empty()
    );
    not_found(store.inbox().get(org_b.id, entry.id).map(|_| ()));
    assert!(
        store
            .inbox()
            .list_pending(org_b.id, 100)
            .expect("lista")
            .is_empty()
    );
    not_found(store.outbox().get(org_b.id, saida.id).map(|_| ()));
    assert!(
        store
            .outbox()
            .list_pending(org_b.id, 100)
            .expect("lista")
            .is_empty()
    );
    assert!(
        store
            .tokens()
            .list(org_b.id, Page::default())
            .expect("lista")
            .is_empty()
    );
    not_found(store.tokens().get(org_b.id, token.id).map(|_| ()));
    assert!(
        store
            .audit()
            .list(org_b.id, Page::default())
            .expect("lista")
            .is_empty()
    );

    assert_eq!(
        store
            .messages()
            .count_rows(org_a.id, conversation.id)
            .expect("conta"),
        1
    );
    assert_eq!(
        store
            .inbox()
            .list_pending(org_a.id, 100)
            .expect("lista")
            .len(),
        1
    );

    let flow = store
        .flows()
        .create(
            org_a.id,
            NewFlow {
                slug: "agendar_visita".to_owned(),
                name: "Agendar visita".to_owned(),
                description: String::new(),
                rascunho_fonte: String::new(),
            },
        )
        .expect("fluxo");
    store
        .flows()
        .release(org_a.id, agent.id, flow.id)
        .expect("libera ao agente da org A");
    not_found(store.flows().get(org_b.id, flow.id).map(|_| ()));
    assert!(
        store
            .flows()
            .by_slug(org_b.id, "agendar_visita")
            .expect("por slug")
            .is_none()
    );
    assert!(
        store
            .flows()
            .list(org_b.id, &FlowFilter::default(), Page::default())
            .expect("lista")
            .is_empty()
    );
    assert!(
        store
            .flows()
            .list_from_agent(org_b.id, agent.id)
            .expect("lista")
            .is_empty()
    );
    not_found(
        store
            .flows()
            .list_agents_of_flow(org_b.id, flow.id)
            .map(|_| ()),
    );
    not_found(store.flows().release(org_b.id, agent.id, flow.id));
    not_found(
        store
            .flows()
            .update(org_b.id, flow.id, 1, FlowChange::default())
            .map(|_| ()),
    );
    not_found(store.flows().delete(org_b.id, flow.id, true).map(|_| ()));
    store
        .flows()
        .get(org_a.id, flow.id)
        .expect("a org A continua vendo o seu fluxo");

    let flow_version = store
        .flows()
        .get(org_a.id, flow.id)
        .expect("fluxo da org A")
        .version;
    let publication = |expected: u64| RevisionPublication {
        expected,
        fonte: "fluxo(id = \"agendar_visita\")".to_owned(),
        definicao_json: "{}".to_owned(),
        schema_versao: Some(1),
        resolucoes_json: "{}".to_owned(),
        ator_tipo: "cli".to_owned(),
        actor_id: None,
    };
    let revision = store
        .revisions()
        .publish_revision(org_a.id, flow.id, publication(flow_version))
        .expect("publica a revisão 1 na org A");
    not_found(store.revisions().get(org_b.id, flow.id, 1).map(|_| ()));
    not_found(
        store
            .revisions()
            .get_by_id(org_b.id, revision.id)
            .map(|_| ()),
    );
    not_found(store.revisions().current(org_b.id, flow.id).map(|_| ()));
    not_found(
        store
            .revisions()
            .list(org_b.id, flow.id, None, 10)
            .map(|_| ()),
    );
    not_found(
        store
            .revisions()
            .publish_revision(org_b.id, flow.id, publication(flow_version + 1))
            .map(|_| ()),
    );
    assert_eq!(
        store
            .revisions()
            .list(org_a.id, flow.id, None, 10)
            .expect("lista da org A")
            .len(),
        1,
        "a org A continua com a sua revisão"
    );

    let case = store
        .cases()
        .create(
            org_a.id,
            NewRecord {
                conversation_id: None,
                agent_id: agent.id,
                canal: "cli".to_owned(),
                peer_externo: Some("cliente-ficticio-a".to_owned()),
            },
        )
        .expect("Ficha da org A");
    not_found(store.cases().get(org_b.id, case.id).map(|_| ()));
    assert!(
        store
            .cases()
            .by_conversation(org_b.id, conversation.id)
            .expect("por conversa")
            .is_none()
    );
    assert!(
        store
            .cases()
            .list(org_b.id, &Default::default(), Page::default())
            .expect("lista")
            .is_empty()
    );
    not_found(
        store
            .cases()
            .events(org_b.id, case.id, None, 10)
            .map(|_| ()),
    );
    not_found(
        store
            .cases()
            .register_event(
                org_b.id,
                case.id,
                NewRecordEvent {
                    execucao_id: None,
                    tipo: RecordEventKind::RecordOpened,
                    ator_tipo: EventActor::System,
                    ator_ref: None,
                    payload_json: "{}".to_owned(),
                    chave_idempotencia: None,
                },
            )
            .map(|_| ()),
    );
    not_found(
        store
            .cases()
            .close_without_conversation(org_b.id, case.id, case.version, "invasão")
            .map(|_| ()),
    );

    let state_initial = || ExecutionState {
        status: ExecutionStatus::Active,
        no_atual: "coleta".to_owned(),
        dados_json: "{}".to_owned(),
        acting_actors: Vec::new(),
        result: None,
        reason: None,
    };
    let opening = || ExecutionStart {
        case: case.id,
        flow: flow.id,
        revision: revision.id,
        actor: EventActor::Attendant,
        ator_ref: Some("token-ficticio"),
        jev_decision: None,
        initial_state: state_initial(),
        inbound_effects: vec![TransitionEffect {
            key: "entrada/0".to_owned(),
            tipo: "mensagem_ao_cliente".to_owned(),
            payload_json: "{\"texto\":\"olá\"}".to_owned(),
        }],
        chave_idempotencia: None,
    };
    let opened = store
        .executions()
        .start_execution(org_a.id, opening())
        .expect("abre a execução na org A");
    let execution = opened.execution.id;
    let effect = opened.effects[0].id;

    not_found(store.executions().get(org_b.id, execution).map(|_| ()));
    assert!(
        store
            .executions()
            .active_of_case(org_b.id, case.id)
            .expect("ativa")
            .is_none()
    );
    assert!(
        store
            .executions()
            .from_case(org_b.id, case.id, 10)
            .expect("da ficha")
            .is_empty()
    );
    not_found(store.executions().stages(org_b.id, execution).map(|_| ()));
    not_found(store.executions().effects(org_b.id, execution).map(|_| ()));
    assert!(
        store
            .executions()
            .active_by_revision(org_b.id, flow.id)
            .expect("ativas por revisão")
            .is_empty()
    );
    not_found(
        store
            .executions()
            .start_execution(org_b.id, opening())
            .map(|_| ()),
    );
    not_found(
        store
            .executions()
            .apply_transition(
                org_b.id,
                execution,
                opened.execution.version,
                &FlowTransition {
                    new_state: state_initial(),
                    events: Vec::new(),
                    effects: Vec::new(),
                    closes_service: false,
                    repeated: false,
                    rule_error: false,
                },
                "chave-invasora",
                TransitionExtras::default(),
            )
            .map(|_| ()),
    );
    assert!(
        store
            .executions()
            .opening_already_done(org_b.id, case.id, flow.id, "chave-invasora")
            .expect("abertura já feita")
            .is_none()
    );
    assert!(
        !store
            .executions()
            .transition_already_applied(org_b.id, execution, "chave-invasora")
            .expect("transição já aplicada")
    );

    not_found(store.effects().get(org_b.id, effect).map(|_| ()));
    assert!(
        store
            .effects()
            .list_pending(org_b.id, 100)
            .expect("pendentes")
            .is_empty()
    );
    assert!(
        store.effects().start_send(org_b.id, effect).is_err(),
        "a org B não pode começar a entrega de um efeito da org A"
    );
    store
        .effects()
        .confirm_send(org_b.id, effect)
        .expect("confirmar sem linha não é erro de banco");
    store
        .effects()
        .fail_send(org_b.id, effect, "invasão", true, None)
        .expect("falhar sem linha não é erro de banco");
    store
        .effects()
        .adiar(org_b.id, effect, 1_700_000_000_000)
        .expect("adiar sem linha não é erro de banco");
    let intacto = store
        .effects()
        .get(org_a.id, effect)
        .expect("o efeito da org A continua lá");
    assert_eq!(intacto.attempts, 0);
    assert_eq!(intacto.ultimo_erro, None);
    assert_eq!(intacto.proxima_tentativa, None);
    assert_eq!(intacto.enviado_em, None);

    assert_eq!(
        store
            .executions()
            .from_case(org_a.id, case.id, 10)
            .expect("da ficha na org A")
            .len(),
        1
    );
    assert_eq!(
        store
            .cases()
            .events(org_a.id, case.id, None, 50)
            .expect("eventos da org A")
            .len(),
        2,
        "ficha_aberta + fluxo_iniciado"
    );
    assert_eq!(
        store
            .effects()
            .list_pending(org_a.id, 100)
            .expect("pendentes da org A")
            .len(),
        1
    );
}

#[test]
fn version_conflict_returns_typed_error_in_all_editable_entities() {
    let scenario = Scenario::new("conflito");
    let store = scenario.store();
    let org = create_org(store, "org-conflito");
    let agent = create_agent(store, &org, "agente-conflito");
    let category = store
        .categories()
        .create(
            org.id,
            NewCategory {
                name: "Categoria".to_owned(),
                description: String::new(),
            },
        )
        .expect("categoria");
    let departamento = store
        .departments()
        .create(
            org.id,
            NewDepartment {
                name: "Departamento".to_owned(),
                description: String::new(),
            },
        )
        .expect("departamento");
    let conversation = create_conversation(store, &org, &agent);
    store
        .instructions()
        .save(org.id, agent.id, 0, "{}", "# v1", true)
        .expect("instruções");

    let checks_conflict = |resultado: Result<(), poph_store::StoreError>| match resultado {
        Err(error) if error.is_version_conflict() => {}
        another => panic!("esperado conflito de versão, veio {another:?}"),
    };

    checks_conflict(
        store
            .orgs()
            .update(
                org.id,
                org.version + 5,
                OrgChange {
                    name: Some("Outro nome".to_owned()),
                    ..OrgChange::default()
                },
            )
            .map(|_| ()),
    );
    checks_conflict(
        store
            .agents()
            .update(
                org.id,
                agent.id,
                agent.version + 5,
                AgentChange {
                    modelo: Some("outro".to_owned()),
                    ..AgentChange::default()
                },
            )
            .map(|_| ()),
    );
    checks_conflict(
        store
            .categories()
            .update(
                org.id,
                category.id,
                category.version + 5,
                CategoryChange {
                    name: Some("Outra".to_owned()),
                    ..CategoryChange::default()
                },
            )
            .map(|_| ()),
    );
    checks_conflict(
        store
            .departments()
            .update(
                org.id,
                departamento.id,
                departamento.version + 5,
                DepartmentChange {
                    name: Some("Outro".to_owned()),
                    ..DepartmentChange::default()
                },
            )
            .map(|_| ()),
    );
    checks_conflict(
        store
            .conversations()
            .change_status(
                org.id,
                conversation.id,
                conversation.version + 5,
                ConversationStatus::Closed,
                None,
            )
            .map(|_| ()),
    );
    checks_conflict(
        store
            .instructions()
            .save(org.id, agent.id, 9, "{}", "# v2", true)
            .map(|_| ()),
    );

    let updated = store
        .agents()
        .update(
            org.id,
            agent.id,
            agent.version,
            AgentChange {
                modelo: Some("gemini-3.8-flash-tiered".to_owned()),
                status: Some(Status::Inactive),
                ..AgentChange::default()
            },
        )
        .expect("atualiza");
    assert_eq!(updated.version, agent.version + 1);
    assert_eq!(updated.status, Status::Inactive);
}

#[test]
fn message_repeated_by_external_id_does_not_duplicate() {
    let scenario = Scenario::new("mensagem-idempotente");
    let store = scenario.store();
    let org = create_org(store, "org-msg");
    let agent = create_agent(store, &org, "agente-msg");
    let conversation = create_conversation(store, &org, &agent);

    let fresh = || NewMessage {
        conversation_id: conversation.id,
        direction: Direction::Inbound,
        role: Role::User,
        origin: poph_store::Origin::Agente,
        author_name: None,
        texto: "bom dia".to_owned(),
        attachments_json: "[]".to_owned(),
        external_id: Some("wamid-42".to_owned()),
        turn_id: None,
    };
    let first = store.messages().write(org.id, fresh()).expect("grava");
    let second = store.messages().write(org.id, fresh()).expect("regrava");
    assert_eq!(first.id, second.id, "a segunda gravação devolve a mesma");
    assert_eq!(
        store
            .messages()
            .count_rows(org.id, conversation.id)
            .expect("conta"),
        1
    );
}

#[test]
fn entry_and_output_are_idempotent_by_prd_key() {
    let scenario = Scenario::new("caixas");
    let store = scenario.store();
    let org = create_org(store, "org-caixas");
    let agent = create_agent(store, &org, "agente-caixas");
    let conversation = create_conversation(store, &org, &agent);
    let integration = IntegrationId::generate();

    let entry = || NewInboxItem {
        integration_id: integration,
        external_id: "wamid-7".to_owned(),
        event_json: "{\"texto\":\"oi\"}".to_owned(),
    };
    let (first_value, item) = store.inbox().receive(org.id, entry()).expect("entrada");
    assert_eq!(first_value, Received::New);
    let (second_value, repeated_value) = store.inbox().receive(org.id, entry()).expect("reentrega");
    assert_eq!(second_value, Received::Duplicate);
    assert_eq!(repeated_value.id, item.id);
    assert_eq!(
        store
            .inbox()
            .list_pending(org.id, 100)
            .expect("lista")
            .len(),
        1
    );

    store
        .inbox()
        .mark_processed(org.id, item.id, None)
        .expect("processa");
    assert!(
        store
            .inbox()
            .list_pending(org.id, 100)
            .expect("lista")
            .is_empty()
    );

    let turno = store
        .turns()
        .start(org.id, conversation.id, Some(agent.id))
        .expect("turno");
    let saida = || NewOutboxItem {
        conversation_id: conversation.id,
        turn_id: turno.id,
        seq: 0,
        tipo: "text".to_owned(),
        content_json: "{\"texto\":\"olá\"}".to_owned(),
    };
    let (new, output_item) = store.outbox().enqueue(org.id, saida()).expect("saída");
    assert!(new);
    let (new_again, repeated_value) = store
        .outbox()
        .enqueue(org.id, saida())
        .expect("saída repetida");
    assert!(!new_again);
    assert_eq!(repeated_value.id, output_item.id);

    store
        .outbox()
        .change_state(org.id, output_item.id, OutboxState::Sending, None)
        .expect("enviando");
    assert_eq!(
        store
            .outbox()
            .list_pending(org.id, 100)
            .expect("lista")
            .len(),
        1
    );
    store
        .outbox()
        .change_state(org.id, output_item.id, OutboxState::Sent, None)
        .expect("enviado");
    assert!(
        store
            .outbox()
            .list_pending(org.id, 100)
            .expect("lista")
            .is_empty()
    );
}

#[test]
fn turn_keeps_why_and_audit_keeps_who() {
    let scenario = Scenario::new("turno");
    let store = scenario.store();
    let org = create_org(store, "org-turno");
    let agent = create_agent(store, &org, "agente-turno");
    let conversation = create_conversation(store, &org, &agent);

    let turno = store
        .turns()
        .start(org.id, conversation.id, Some(agent.id))
        .expect("turno");
    let fim = poph_store::TurnFinalization {
        state: Some(poph_store::TurnState::Completed),
        times_json: Some("{\"llm_ms\":6500}".to_owned()),
        input_tokens: Some(1200),
        output_tokens: Some(80),
        ..poph_store::TurnFinalization::default()
    };
    let closed = store
        .turns()
        .finalise(org.id, turno.id, fim)
        .expect("fecha");
    assert_eq!(closed.state, poph_store::TurnState::Completed);
    assert!(closed.finished_at.is_some());

    let registro = store
        .audit()
        .register(
            Some(org.id),
            NewAuditEntry {
                ator_tipo: "admin_token".to_owned(),
                actor_id: Some("token-1".to_owned()),
                acao: "update".to_owned(),
                entity: "agent".to_owned(),
                entity_id: Some(agent.id.to_string()),
                detail_json: "{\"campo\":\"modelo\"}".to_owned(),
            },
        )
        .expect("auditoria");
    assert_eq!(registro.acao, "update");
    assert_eq!(
        store
            .audit()
            .list(org.id, Page::default())
            .expect("lista")
            .len(),
        1
    );
}

#[test]
fn ten_thousand_concurrent_writes_without_busy_and_without_loss() {
    let scenario = Scenario::new("concorrencia");
    let store = scenario.store();
    let org = create_org(store, "org-concorrencia");
    let agent = create_agent(store, &org, "agente-concorrencia");
    let conversation = create_conversation(store, &org, &agent);

    const THREADS: usize = 10;
    const BY_THREAD: usize = 1000;
    let barreira = Arc::new(Barrier::new(THREADS));
    let org_id: OrgId = org.id;
    let conversation_id = conversation.id;
    let mut tasks = Vec::with_capacity(THREADS);
    for thread_id in 0..THREADS {
        let store = scenario.shared();
        let barreira = Arc::clone(&barreira);
        tasks.push(thread::spawn(move || {
            barreira.wait();
            for index in 0..BY_THREAD {
                store
                    .messages()
                    .write(
                        org_id,
                        NewMessage {
                            conversation_id,
                            direction: Direction::Inbound,
                            role: Role::User,
                            texto: format!("mensagem {thread_id}-{index}"),
                            attachments_json: "[]".to_owned(),
                            external_id: None,
                            turn_id: None,
                            origin: poph_store::Origin::Agente,
                            author_name: None,
                        },
                    )
                    .expect("gravação concorrente não pode falhar");
            }
        }));
    }
    for task in tasks {
        task.join().expect("thread concluiu");
    }

    assert_eq!(
        store
            .messages()
            .count_rows(org.id, conversation.id)
            .expect("conta"),
        (THREADS * BY_THREAD) as u64,
        "nenhuma escrita pode ser perdida"
    );
    assert!(store.broken_foreign_keys().expect("fk check").is_empty());
}

#[test]
fn invalid_entry_does_not_drop_batch() {
    let scenario = Scenario::new("erro-no-lote");
    let store = scenario.store();
    let org = create_org(store, "org-erro");

    let error = store
        .orgs()
        .create(NewOrg {
            name: "Repetida".to_owned(),
            slug: "org-erro".to_owned(),
            config_json: String::new(),
        })
        .expect_err("slug repetido deve falhar");
    assert!(
        matches!(
            error,
            poph_store::StoreError::Domain(DomainError::Validation { .. })
        ),
        "veio {error:?}"
    );

    let other = create_org(store, "org-erro-2");
    assert_eq!(other.status, Status::Active);
    assert_eq!(store.orgs().list(Page::default()).expect("lista").len(), 2);

    let missing = store
        .agents()
        .get(org.id, poph_types::AgentId::generate())
        .expect_err("agente inexistente");
    assert!(missing.is_not_found(), "veio {missing:?}");
}

#[test]
fn tokens_appear_only_once_and_revoke_removes_from_authentication() {
    let scenario = Scenario::new("tokens");
    let store = scenario.store();
    let org = create_org(store, "org-token");
    let token = store
        .tokens()
        .create(Some(org.id), "CI", "hash-unico")
        .expect("token");
    assert!(
        store
            .tokens()
            .by_hash("hash-unico")
            .expect("busca")
            .is_some()
    );
    store.tokens().revoke(org.id, token.id).expect("revoga");
    assert!(
        store
            .tokens()
            .by_hash("hash-unico")
            .expect("busca")
            .is_none(),
        "revogado não autentica"
    );
    assert!(
        matches!(
            store.inbox().get(org.id, InboxId::generate()).map(|_| ()),
            Err(poph_store::StoreError::Domain(DomainError::NotFound { .. }))
        ),
        "item de entrada inexistente precisa dar não encontrado"
    );
}

#[test]
fn link_between_orgs_is_refused_and_writes_nothing() {
    let scenario = Scenario::new("vinculo-entre-orgs");
    let store = scenario.store();
    let org_a = create_org(store, "org-a");
    let org_b = create_org(store, "org-b");
    let agent_a = create_agent(store, &org_a, "agente-a");
    let agent_b = create_agent(store, &org_b, "agente-b");
    let department_b = store
        .departments()
        .create(
            org_b.id,
            NewDepartment {
                name: "Departamento B".to_owned(),
                description: String::new(),
            },
        )
        .expect("departamento de B");
    let category_b = store
        .categories()
        .create(
            org_b.id,
            NewCategory {
                name: "Categoria B".to_owned(),
                description: String::new(),
            },
        )
        .expect("categoria de B");

    let refused = store
        .agents()
        .update(
            org_a.id,
            agent_a.id,
            agent_a.version,
            AgentChange {
                default_department: Some(department_b.id),
                ..AgentChange::default()
            },
        )
        .expect_err("departamento de outra org");
    assert!(refused.is_not_found(), "veio {refused:?}");
    assert_eq!(
        store
            .agents()
            .get(org_a.id, agent_a.id)
            .expect("agente intacto")
            .version,
        agent_a.version,
        "a versão não pode andar numa escrita recusada"
    );

    let refused = store
        .agents()
        .define_categories(org_a.id, agent_a.id, agent_a.version, &[category_b.id])
        .expect_err("categoria de outra org");
    assert!(refused.is_not_found(), "veio {refused:?}");
    assert!(
        store
            .agents()
            .categories(org_a.id, agent_a.id)
            .expect("categorias")
            .is_empty()
    );

    let refused = store
        .conversations()
        .create(
            org_a.id,
            NewConversation {
                agent_id: agent_b.id,
                integration_id: None,
                canal: "cli".to_owned(),
                peer_externo: None,
                target_department: None,
            },
        )
        .expect_err("agente de outra org");
    assert!(
        matches!(refused, poph_store::StoreError::Sqlite(_)),
        "veio {refused:?}"
    );
    assert!(
        store
            .conversations()
            .list(org_a.id, None, Page::default())
            .expect("lista")
            .is_empty(),
        "conversa recusada não pode aparecer"
    );

    let conversation_b = create_conversation(store, &org_b, &agent_b);
    let refused = store
        .messages()
        .write(
            org_a.id,
            NewMessage {
                conversation_id: conversation_b.id,
                direction: Direction::Inbound,
                role: Role::User,
                texto: "invasão".to_owned(),
                attachments_json: "[]".to_owned(),
                external_id: None,
                turn_id: None,
                origin: poph_store::Origin::Agente,
                author_name: None,
            },
        )
        .expect_err("conversa de outra org");
    assert!(
        matches!(refused, poph_store::StoreError::Sqlite(_)),
        "veio {refused:?}"
    );
    assert_eq!(
        store
            .messages()
            .count_rows(org_b.id, conversation_b.id)
            .expect("conta"),
        0
    );

    let conversation_a = create_conversation(store, &org_a, &agent_a);
    let refused = store
        .turns()
        .start(org_a.id, conversation_a.id, Some(agent_b.id))
        .expect_err("agente de outra org");
    assert!(refused.is_not_found(), "veio {refused:?}");
}

#[test]
fn deleting_org_takes_its_data_along() {
    let scenario = Scenario::new("cascata");
    let store = scenario.store();
    let org = create_org(store, "org-cascata");
    let another = create_org(store, "org-fica");
    store
        .departments()
        .create(
            org.id,
            NewDepartment {
                name: "Dep".to_owned(),
                description: String::new(),
            },
        )
        .expect("departamento");
    let category = store
        .categories()
        .create(
            org.id,
            NewCategory {
                name: "Cat".to_owned(),
                description: String::new(),
            },
        )
        .expect("categoria");
    let agent = create_agent(store, &org, "agente-cascata");
    store
        .agents()
        .define_categories(org.id, agent.id, agent.version, &[category.id])
        .expect("vínculo");
    store
        .instructions()
        .save(org.id, agent.id, 0, "{}", "# A", true)
        .expect("instruções");
    let conversation = create_conversation(store, &org, &agent);
    write_message(store, &org, &conversation, "oi");
    let turno = store
        .turns()
        .start(org.id, conversation.id, Some(agent.id))
        .expect("turno");

    let refusal = store.orgs().delete(org.id).expect_err("recusa com agentes");
    assert!(matches!(
        refusal,
        StoreError::Domain(DomainError::Conflict { .. })
    ));
    store.orgs().get(org.id).expect("a org continua");

    let agents_deleted = store.orgs().delete_forced(org.id).expect("exclui a org");
    assert_eq!(agents_deleted, 1);
    assert!(store.orgs().get(org.id).is_err());
    assert!(
        store
            .agents()
            .list(org.id, Page::default())
            .expect("lista")
            .is_empty()
    );
    assert_eq!(
        store
            .conversations()
            .list(org.id, None, Page::default())
            .expect("lista")
            .len(),
        0
    );
    assert!(store.turns().get(org.id, turno.id).is_err());
    assert!(
        store
            .departments()
            .list(org.id, Page::default())
            .expect("lista")
            .is_empty()
    );
    assert!(store.broken_foreign_keys().expect("fk check").is_empty());

    store.orgs().get(another.id).expect("a outra org fica");
}

#[test]
fn inactive_agent_opens_no_turn_but_stays_listed_and_editable() {
    let scenario = Scenario::new("agente-inativo");
    let store = scenario.store();
    let org = create_org(store, "org-inativa");
    let agent = create_agent(store, &org, "agente-inativo");

    assert!(agent.can_open_turn());
    store
        .agents()
        .get_for_turn(org.id, agent.id)
        .expect("agente ativo abre turno");

    let inactive = store
        .agents()
        .update(
            org.id,
            agent.id,
            agent.version,
            AgentChange {
                status: Some(Status::Inactive),
                ..AgentChange::default()
            },
        )
        .expect("inativa o agente");
    assert!(!inactive.can_open_turn(), "D-05: inativo não abre turno");
    match store.agents().get_for_turn(org.id, agent.id) {
        Err(StoreError::Domain(DomainError::AgentInactive { id, status })) => {
            assert_eq!(id, agent.id.to_string());
            assert_eq!(status, "inactive");
        }
        another => panic!("esperava AgenteInativo, veio {another:?}"),
    }

    assert!(
        store
            .agents()
            .list(org.id, Page::default())
            .expect("lista")
            .iter()
            .any(|item| item.id == agent.id)
    );
    assert!(
        store
            .agents()
            .list_active(org.id)
            .expect("lista ativos")
            .is_empty()
    );
}

#[test]
fn instructions_of_one_org_neither_appear_nor_are_deleted_in_another() {
    let scenario = Scenario::new("instrucoes-org");
    let store = scenario.store();
    let org_a = create_org(store, "org-a-instrucoes");
    let org_b = create_org(store, "org-b-instrucoes");
    let agent_a = create_agent(store, &org_a, "agente-a-instrucoes");
    let agent_b = create_agent(store, &org_b, "agente-b-instrucoes");

    store
        .instructions()
        .save(org_a.id, agent_a.id, 0, "{}", "# A", true)
        .expect("instruções da org A");
    store
        .instructions()
        .save(org_b.id, agent_b.id, 0, "{}", "# B", true)
        .expect("instruções da org B");

    assert!(store.instructions().get(org_b.id, agent_a.id).is_err());
    assert!(store.instructions().get(org_a.id, agent_b.id).is_err());

    let only_b = store
        .instructions()
        .list(org_b.id, Page::default())
        .expect("lista B");
    assert_eq!(only_b.len(), 1);
    assert_eq!(only_b[0].agent_id, agent_b.id);
    assert_eq!(only_b[0].markdown, "# B");

    assert!(store.instructions().delete(org_b.id, agent_a.id).is_err());
    assert!(store.instructions().get(org_a.id, agent_a.id).is_ok());

    let first = store
        .instructions()
        .get(org_a.id, agent_a.id)
        .expect("instruções da org A");
    let second = store
        .instructions()
        .save(org_a.id, agent_a.id, first.version, "{}", "# A2", true)
        .expect("atualiza");
    assert_eq!(second.version, first.version + 1);
    let conflict = store
        .instructions()
        .save(org_a.id, agent_a.id, first.version, "{}", "# A3", true)
        .expect_err("versão velha deve conflitar");
    assert!(conflict.is_version_conflict(), "{conflict:?}");
}

#[test]
fn instructions_version_zero_creates_and_old_version_does_not_recreate() {
    let scenario = Scenario::new("instrucoes-versao-ausente");
    let store = scenario.store();
    let org = create_org(store, "org-instrucoes-versao");
    let agent = create_agent(store, &org, "agente-instrucoes-versao");
    let repo = store.instructions();
    let first = repo
        .save(org.id, agent.id, 0, "{}", "primeira", true)
        .unwrap();
    assert!(
        repo.save(org.id, agent.id, 0, "{}", "duplicada", true)
            .unwrap_err()
            .is_version_conflict()
    );
    repo.delete(org.id, agent.id).unwrap();
    let error = repo
        .save(org.id, agent.id, first.version, "{}", "antiga", true)
        .unwrap_err();
    assert!(matches!(
        error,
        StoreError::Domain(DomainError::VersionConflict {
            expected: 1,
            current: 0,
            ..
        })
    ));
    assert!(repo.get(org.id, agent.id).is_err());
    assert_eq!(
        repo.save(org.id, agent.id, 0, "{}", "nova", true)
            .unwrap()
            .markdown,
        "nova"
    );
}

#[test]
fn orphan_turns_closes_only_those_started_before_boot() {
    let scenario = Scenario::new("orfao");
    let store = scenario.store();
    let org = create_org(store, "org-orfao");
    let agent = create_agent(store, &org, "agente-orfao");
    let conversation = create_conversation(store, &org, &agent);

    let before = store
        .turns()
        .start(org.id, conversation.id, Some(agent.id))
        .expect("turno do processo anterior");
    let cutoff = store.now() + 1;
    std::thread::sleep(std::time::Duration::from_millis(5));
    let later = store
        .turns()
        .start(org.id, conversation.id, Some(agent.id))
        .expect("turno desta instância");

    let closed_rows = store
        .turns()
        .fail_orphans(org.id, "interrompido", cutoff)
        .expect("fecha os órfãos");
    assert_eq!(closed_rows, 1, "só o turno anterior à subida foi fechado");

    let orphan = store.turns().get(org.id, before.id).expect("órfão");
    assert_eq!(orphan.state, poph_store::TurnState::Failed);
    assert_eq!(orphan.error.as_deref(), Some("interrompido"));
    assert!(orphan.finished_at.is_some());

    let live = store.turns().get(org.id, later.id).expect("turno vivo");
    assert_eq!(
        live.state,
        poph_store::TurnState::Running,
        "turno começado depois da subida não é tocado"
    );

    let other = create_org(store, "org-vizinha");
    let agent_other = create_agent(store, &other, "agente-vizinho");
    let conversation_other = create_conversation(store, &other, &agent_other);
    let from_other = store
        .turns()
        .start(other.id, conversation_other.id, Some(agent_other.id))
        .expect("turno da vizinha");
    assert_eq!(
        store
            .turns()
            .fail_orphans(org.id, "interrompido", i64::MAX)
            .expect("fecha de novo"),
        1,
        "a varredura de uma org não fecha turno de outra"
    );
    assert_eq!(
        store
            .turns()
            .get(other.id, from_other.id)
            .expect("turno da vizinha")
            .state,
        poph_store::TurnState::Running
    );
}

#[test]
fn response_and_output_item_enter_same_write() {
    let scenario = Scenario::new("saida-junta");
    let store = scenario.store();
    let org = create_org(store, "org-saida-junta");
    let agent = create_agent(store, &org, "agente-saida-junta");
    let conversation = create_conversation(store, &org, &agent);
    let turno = store
        .turns()
        .start(org.id, conversation.id, Some(agent.id))
        .expect("turno");

    let (mensagem, item) = store
        .messages()
        .write_with_output(
            org.id,
            NewMessage {
                conversation_id: conversation.id,
                direction: Direction::Outbound,
                role: Role::Assistant,
                texto: "resposta do agente".to_owned(),
                attachments_json: "[]".to_owned(),
                external_id: None,
                turn_id: Some(turno.id),
                origin: poph_store::Origin::Agente,
                author_name: None,
            },
            NewOutboxItem {
                conversation_id: conversation.id,
                turn_id: turno.id,
                seq: 0,
                tipo: "text".to_owned(),
                content_json: "{\"texto\":\"resposta do agente\"}".to_owned(),
            },
        )
        .expect("grava mensagem e saída juntas");

    assert_eq!(mensagem.turn_id, Some(turno.id));
    assert_eq!(item.turn_id, turno.id);
    assert_eq!(item.state, OutboxState::Pending);
    assert_eq!(
        store
            .messages()
            .list(org.id, conversation.id, Page::with_limit(10))
            .expect("mensagens")
            .len(),
        1
    );
    assert_eq!(
        store.outbox().list_pending(org.id, 10).expect("fila").len(),
        1
    );

    let (_, repeated_value) = store
        .messages()
        .write_with_output(
            org.id,
            NewMessage {
                conversation_id: conversation.id,
                direction: Direction::Outbound,
                role: Role::Assistant,
                texto: "resposta do agente".to_owned(),
                attachments_json: "[]".to_owned(),
                external_id: None,
                turn_id: Some(turno.id),
                origin: poph_store::Origin::Agente,
                author_name: None,
            },
            NewOutboxItem {
                conversation_id: conversation.id,
                turn_id: turno.id,
                seq: 0,
                tipo: "text".to_owned(),
                content_json: "{\"texto\":\"resposta do agente\"}".to_owned(),
            },
        )
        .expect("repete");
    assert_eq!(
        repeated_value.id, item.id,
        "o item da fila é o mesmo (turno + seq)"
    );
    assert_eq!(
        store.outbox().list_pending(org.id, 10).expect("fila").len(),
        1
    );
}
